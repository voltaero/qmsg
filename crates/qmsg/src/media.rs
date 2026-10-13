use qmsg_types::{Content, Media, MediaSource, Message};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 16384;
pub const MAX_PIXELS: u64 = 40_000_000;
pub const MAX_SIDE: u32 = 1600;
pub const TIMEOUT: Duration = Duration::from_secs(20);
pub const MAX_REDIRECTS: usize = 3;
pub const CACHE_BYTES: usize = 96 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
}
impl Kind {
    pub fn limit(self) -> u64 {
        match self {
            Kind::Image => MAX_IMAGE_BYTES,
            Kind::Video => MAX_VIDEO_BYTES,
        }
    }
}

fn mime_kind(mime: Option<&str>) -> Option<Kind> {
    let mime = mime?.split(';').next()?.trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" => Some(Kind::Image),
        _ if mime.starts_with("video/") => Some(Kind::Video),
        _ => None,
    }
}

pub fn kind(content: &Content) -> Option<(Kind, &Media)> {
    match content {
        Content::Image(media) => match mime_kind(media.mime.as_deref()) {
            Some(Kind::Video) => Some((Kind::Video, media)),
            _ => Some((Kind::Image, media)),
        },
        Content::Video(media) => Some((Kind::Video, media)),
        Content::File(media) => mime_kind(media.mime.as_deref()).map(|kind| (kind, media)),
        _ => None,
    }
}

pub fn expired(media: &Media, now_ms: u64) -> bool {
    media.metadata.expires_at_ms.is_some_and(|at| at <= now_ms)
}

pub fn too_large(limit: u64) -> String {
    format!("Media is larger than {} MiB", limit / (1024 * 1024))
}

/// Rejects media that cannot load before anything is downloaded.
pub fn check(media: &Media, kind: Kind, now_ms: u64) -> Result<(), String> {
    if expired(media, now_ms) {
        return Err("Media link expired; refreshing will renew it".into());
    }
    if media.size().is_some_and(|size| size > kind.limit()) {
        return Err(too_large(kind.limit()));
    }
    Ok(())
}

pub fn blob_ids(message: &Message) -> impl Iterator<Item = &str> {
    message.content.iter().filter_map(|content| match content {
        Content::Image(media)
        | Content::Video(media)
        | Content::Audio(media)
        | Content::File(media) => match &media.source {
            MediaSource::Blob(id) => Some(id.as_str()),
            _ => None,
        },
        _ => None,
    })
}

#[derive(Debug, Clone)]
pub enum Loaded {
    Image(fxcode::Image),
    Video(Arc<VideoFile>),
}

/// A downloaded video in a private directory, removed on drop.
#[derive(Debug)]
pub struct VideoFile {
    dir: PathBuf,
    path: PathBuf,
}
impl VideoFile {
    pub fn create(bytes: &[u8], mime: Option<&str>) -> Result<Self, String> {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(std::env::temp_dir);
        Self::create_in(&base, bytes, mime).map_err(|_| "Could not store video".into())
    }
    fn create_in(base: &Path, bytes: &[u8], mime: Option<&str>) -> std::io::Result<Self> {
        let dir = base.join(format!("qmsg-media-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let file = Self {
            path: dir.join(video_name(mime)),
            dir,
        };
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file.path)?
            .write_all(bytes)?;
        Ok(file)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for VideoFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

fn video_name(mime: Option<&str>) -> &'static str {
    let mime = mime
        .and_then(|m| m.split(';').next())
        .map(|m| m.trim().to_ascii_lowercase());
    match mime.as_deref() {
        Some("video/mp4") => "video.mp4",
        Some("video/webm") => "video.webm",
        Some("video/quicktime") => "video.mov",
        Some("video/x-matroska") => "video.mkv",
        Some("video/ogg") => "video.ogv",
        Some("image/gif") => "video.gif",
        _ => "video",
    }
}

pub fn decode_image(bytes: &[u8]) -> Result<fxcode::Image, String> {
    use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits};
    let failed = |error: ImageError| match error {
        ImageError::Limits(_) => "Image is too large".to_string(),
        _ => "Could not decode image".to_string(),
    };
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Could not decode image")?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP)
    ) {
        return Err("Unsupported image format".into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(failed)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err("Could not decode image".into());
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION || width as u64 * height as u64 > MAX_PIXELS
    {
        return Err("Image is too large".into());
    }
    let mut image = DynamicImage::from_decoder(decoder).map_err(failed)?;
    if width.max(height) > MAX_SIDE {
        image = image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let rgba = image.into_rgba8();
    fxcode::Image::from_rgba(rgba.width(), rgba.height(), rgba.into_raw())
        .map_err(|_| "Could not decode image".into())
}

/// Decodes an image or stores a video. Blocking.
pub fn load(bytes: &[u8], mime: Option<&str>, kind: Kind) -> Result<Loaded, String> {
    if bytes.len() as u64 > kind.limit() {
        return Err(too_large(kind.limit()));
    }
    match kind {
        Kind::Image => decode_image(bytes).map(Loaded::Image),
        Kind::Video => VideoFile::create(bytes, mime).map(|file| Loaded::Video(Arc::new(file))),
    }
}

/// Loads inline media without a backend, as in demo mode.
pub fn load_local(media: &Media, kind: Kind) -> Result<Loaded, String> {
    match &media.source {
        MediaSource::Bytes(bytes) => load(bytes, media.mime.as_deref(), kind),
        _ => Err("Media is unavailable offline".into()),
    }
}

fn allowed(url: &reqwest::Url) -> bool {
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == "https" || (url.scheme() == "http" && loopback))
}

/// A client for public media downloads: no cookies, credentials or referer.
pub fn http_client() -> Result<reqwest::Client, String> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "Could not set up TLS")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if !allowed(attempt.url()) {
                attempt.error("insecure redirect")
            } else {
                attempt.follow()
            }
        }))
        .referer(false)
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .user_agent("qmsg")
        .build()
        .map_err(|_| "Could not set up downloads".into())
}

// Messages never include the URL: signed query strings are secrets.
fn download_error(error: reqwest::Error) -> String {
    let error = error.without_url();
    if error.is_timeout() {
        "Download timed out".into()
    } else if error.is_redirect() {
        "Download redirected too often or to an insecure address".into()
    } else if error.is_connect() {
        "Could not connect to media server".into()
    } else {
        "Download failed".into()
    }
}

pub async fn fetch_url(http: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(url).map_err(|_| "Invalid media link")?;
    if !allowed(&url) {
        return Err("Media link must use HTTPS".into());
    }
    let mut response = http.get(url).send().await.map_err(download_error)?;
    if !response.status().is_success() {
        return Err(format!(
            "Download failed: HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(too_large(limit));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(download_error)? {
        if (body.len() + chunk.len()) as u64 > limit {
            return Err(too_large(limit));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Decoded images, least recently used first out, within [`CACHE_BYTES`].
#[derive(Default)]
pub struct Cache {
    entries: HashMap<String, (fxcode::Image, u64)>,
    order: BTreeMap<u64, String>,
    tick: u64,
    bytes: usize,
}
impl Cache {
    pub fn get(&mut self, key: &str) -> Option<&fxcode::Image> {
        let (image, used) = self.entries.get_mut(key)?;
        let key = self.order.remove(used)?;
        self.tick += 1;
        *used = self.tick;
        self.order.insert(self.tick, key);
        Some(image)
    }
    pub fn insert(&mut self, key: String, image: fxcode::Image) {
        let size = image_bytes(&image);
        if let Some((old, used)) = self.entries.remove(&key) {
            self.order.remove(&used);
            self.bytes -= image_bytes(&old);
        }
        if size > CACHE_BYTES {
            return;
        }
        while self.bytes + size > CACHE_BYTES {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some((old, _)) = self.entries.remove(&oldest) {
                self.bytes -= image_bytes(&old);
            }
        }
        self.tick += 1;
        self.bytes += size;
        self.order.insert(self.tick, key.clone());
        self.entries.insert(key, (image, self.tick));
    }
}
fn image_bytes(image: &fxcode::Image) -> usize {
    image
        .dimensions()
        .map_or(0, |(w, h)| w as usize * h as usize * 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbaImage};
    use qmsg_types::{ChannelRef, MediaMetadata};
    use std::os::unix::fs::PermissionsExt;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn encode(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }

    fn media(source: MediaSource, expires_at_ms: Option<u64>) -> Media {
        Media {
            name: Some("../../evil.sh".into()),
            mime: Some("video/mp4".into()),
            size: None,
            source,
            metadata: MediaMetadata {
                expires_at_ms,
                ..Default::default()
            },
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A valid PNG header with no pixel data.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for chunk in [ihdr, b"IDAT".to_vec(), b"IEND".to_vec()] {
            png.extend((chunk.len() as u32 - 4).to_be_bytes());
            png.extend(&chunk);
            png.extend(crc32(&chunk).to_be_bytes());
        }
        png
    }

    #[test]
    fn decodes_png_and_first_gif_frame_as_rgba() {
        let image = decode_image(&encode(3, 2, ImageFormat::Png)).unwrap();
        assert_eq!(image.dimensions(), Some((3, 2)));
        let image = decode_image(&encode(4, 5, ImageFormat::Gif)).unwrap();
        assert_eq!(image.dimensions(), Some((4, 5)));
        let mut cache = Cache::default();
        cache.insert("gif".into(), image);
        assert_eq!(cache.bytes, 4 * 5 * 4);
    }

    #[test]
    fn downscales_large_images() {
        let image = decode_image(&encode(4096, 1024, ImageFormat::Png)).unwrap();
        assert_eq!(image.dimensions(), Some((1600, 400)));
    }

    #[test]
    fn rejects_oversized_headers_before_decoding() {
        // Only a header: a full decode would fail with a different error.
        assert_eq!(
            decode_image(&png_header(20_000, 10)).unwrap_err(),
            "Image is too large"
        );
        assert_eq!(
            decode_image(&png_header(10_000, 5_000)).unwrap_err(),
            "Image is too large"
        );
        assert_eq!(
            decode_image(&png_header(100, 100)).unwrap_err(),
            "Could not decode image"
        );
        assert_eq!(
            decode_image(b"#!/bin/sh\n").unwrap_err(),
            "Unsupported image format"
        );
    }

    #[test]
    fn kinds_follow_content_and_mime() {
        let mut file = media(MediaSource::Url("https://a/b".into()), None);
        assert_eq!(kind(&Content::Video(file.clone())).unwrap().0, Kind::Video);
        assert_eq!(kind(&Content::Image(file.clone())).unwrap().0, Kind::Video);
        assert_eq!(kind(&Content::File(file.clone())).unwrap().0, Kind::Video);
        assert!(kind(&Content::Audio(file.clone())).is_none());
        file.mime = Some("IMAGE/PNG; q=1".into());
        assert_eq!(kind(&Content::File(file.clone())).unwrap().0, Kind::Image);
        file.mime = Some("image/svg+xml".into());
        assert!(kind(&Content::File(file.clone())).is_none());
        assert_eq!(kind(&Content::Image(file)).unwrap().0, Kind::Image);
    }

    #[test]
    fn expiry_and_size_are_checked_first() {
        let source = MediaSource::Url("https://a/b".into());
        assert!(!expired(&media(source.clone(), None), 10));
        assert!(!expired(&media(source.clone(), Some(11)), 10));
        assert!(expired(&media(source.clone(), Some(10)), 10));
        assert!(
            check(&media(source.clone(), Some(5)), Kind::Video, 10)
                .unwrap_err()
                .contains("expired")
        );
        let mut large = media(source, None);
        large.size = Some(MAX_IMAGE_BYTES + 1);
        assert!(check(&large, Kind::Image, 10).is_err());
        assert!(check(&large, Kind::Video, 10).is_ok());
    }

    #[test]
    fn lists_blob_ids() {
        let message = Message::new(
            "1",
            ChannelRef::new(None, "c"),
            "u",
            0,
            vec![
                Content::Text("hi".into()),
                Content::Image(media(MediaSource::Blob("a".into()), None)),
                Content::File(media(MediaSource::Url("https://x".into()), None)),
                Content::Audio(media(MediaSource::Blob("b".into()), None)),
            ],
        );
        assert_eq!(blob_ids(&message).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn video_files_are_private_and_removed_on_drop() {
        let base = std::env::temp_dir();
        let file = VideoFile::create_in(&base, b"data", Some("video/mp4")).unwrap();
        let dir = file.path().parent().unwrap().to_path_buf();
        assert_eq!(file.path().file_name().unwrap(), "video.mp4");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(file.path()), 0o600);
        assert_eq!(std::fs::read(file.path()).unwrap(), b"data");
        let path = file.path().to_path_buf();
        drop(file);
        assert!(!path.exists() && !dir.exists());
        let loaded =
            load_local(&media(MediaSource::Bytes(b"x".to_vec()), None), Kind::Video).unwrap();
        let Loaded::Video(file) = loaded else {
            panic!()
        };
        assert_eq!(file.path().file_name().unwrap(), "video.mp4");
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let image = |side| {
            fxcode::Image::from_rgba(side, side, vec![0; (side * side * 4) as usize]).unwrap()
        };
        let mut cache = Cache::default();
        let full = CACHE_BYTES / (2048 * 2048 * 4);
        for key in 0..full {
            cache.insert(key.to_string(), image(2048));
        }
        assert!(cache.get("0").is_some());
        cache.insert("e".into(), image(16));
        assert_eq!(cache.entries.len(), full);
        assert!(cache.get("1").is_none());
        assert!(cache.get("0").is_some() && cache.get("e").is_some());
        assert!(cache.bytes <= CACHE_BYTES);
    }

    /// Serves each response once per connection and returns the base URL.
    async fn serve(responses: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(&response).await;
                let _ = stream.shutdown().await;
            }
        });
        format!("http://{address}")
    }

    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response =
            format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
        response.extend_from_slice(body);
        response
    }

    #[tokio::test]
    async fn fetches_bounded_public_downloads() {
        let http = http_client().unwrap();
        let chunked = b"4\r\nabcd\r\n4\r\nefgh\r\n4\r\nijkl\r\n0\r\n\r\n";
        let base = serve(vec![
            response("200 OK", "Content-Length: 5\r\n", b"hello"),
            response("200 OK", "Content-Length: 100\r\n", &[0; 100]),
            response("200 OK", "Transfer-Encoding: chunked\r\n", chunked),
            response("404 Not Found", "Content-Length: 0\r\n", b""),
            response(
                "302 Found",
                "Location: http://192.0.2.1/x?ex=secret\r\nContent-Length: 0\r\n",
                b"",
            ),
        ])
        .await;
        let url = format!("{base}/media.png?ex=secret&sig=abc");
        assert_eq!(fetch_url(&http, &url, 10).await.unwrap(), b"hello");
        let errors = [
            fetch_url(&http, &url, 10).await.unwrap_err(),
            fetch_url(&http, &url, 10).await.unwrap_err(),
            fetch_url(&http, &url, 10).await.unwrap_err(),
            fetch_url(&http, &url, 10).await.unwrap_err(),
            fetch_url(&http, "http://192.0.2.1/a?ex=secret", 10)
                .await
                .unwrap_err(),
            fetch_url(&http, "https://user:pw@example.com/a", 10)
                .await
                .unwrap_err(),
        ];
        assert_eq!(errors[0], too_large(10));
        assert_eq!(errors[1], too_large(10));
        assert_eq!(errors[2], "Download failed: HTTP 404");
        assert!(errors[3].contains("redirect"));
        assert_eq!(errors[4], "Media link must use HTTPS");
        assert_eq!(errors[5], "Media link must use HTTPS");
        for error in errors {
            assert!(
                !error.contains("secret") && !error.contains("192.0.2.1"),
                "{error}"
            );
        }
    }
}
