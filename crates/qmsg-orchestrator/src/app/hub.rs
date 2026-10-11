use super::{
    auth,
    media::{Media, Stored},
    wire,
};
use crate::{Directory, ProviderEvent, ProviderHandle, ProviderId};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tonic::Status;
struct Update {
    sequence: u64,
    provider: ProviderId,
    body: Stored,
    bytes: usize,
}
struct State {
    directory: Directory,
    views: BTreeMap<String, (ProviderId, Stored)>,
    ring: VecDeque<Arc<Update>>,
    sequence: u64,
    bytes: usize,
    statuses: BTreeMap<String, qmsg_types::ProviderStatus>,
}
pub struct Hub {
    pub epoch: String,
    state: Mutex<State>,
    updates: broadcast::Sender<Arc<Update>>,
    barriers: mpsc::Sender<oneshot::Sender<u64>>,
    pub providers: BTreeMap<String, Arc<ProviderHandle>>,
    pub media: Arc<Media>,
}
impl Hub {
    pub fn new(
        providers: Vec<Arc<ProviderHandle>>,
        mut events: mpsc::Receiver<ProviderEvent>,
        media: Arc<Media>,
    ) -> Arc<Self> {
        let (updates, _) = broadcast::channel(128);
        let (barriers, mut requests) = mpsc::channel::<oneshot::Sender<u64>>(64);
        let providers: BTreeMap<_, _> = providers
            .into_iter()
            .map(|p| (p.id().name.clone(), p))
            .collect();
        let hub = Arc::new(Self {
            epoch: auth::random(),
            state: Mutex::new(State {
                directory: Directory::new(),
                views: BTreeMap::new(),
                ring: VecDeque::new(),
                sequence: 0,
                bytes: 0,
                statuses: BTreeMap::new(),
            }),
            updates,
            barriers,
            providers,
            media,
        });
        for p in hub.providers.values() {
            hub.refresh(p, &mut hub.state.lock().unwrap());
        }
        let weak = Arc::downgrade(&hub);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
            let mut events_open = true;
            loop {
                tokio::select! {
                event=events.recv(),if events_open=>{match event {Some(event)=>if let Some(hub)=weak.upgrade(){hub.apply(event);}else{break},None=>events_open=false}},
                request=requests.recv()=>{let Some(request)=request else {break};let Some(hub)=weak.upgrade()else{break};while let Ok(event)=events.try_recv(){hub.apply(event);}let sequence=hub.state.lock().unwrap().sequence;let _=request.send(sequence);},
                _=tick.tick()=>{let Some(hub)=weak.upgrade()else{break};hub.media.cleanup();}
                }
            }
        });
        hub
    }
    fn refresh(&self, p: &Arc<ProviderHandle>, state: &mut State) {
        let id = p.id();
        let organizations: Vec<_> = state.directory.organizations(id).collect();
        let standalone = state.directory.scope(id, None);
        let value = serde_json::json!({"provider":id,"status":state.statuses.get(&id.name).cloned().unwrap_or(qmsg_types::ProviderStatus::Connecting),"organizations":organizations,"standalone":standalone});
        if let Ok(body) = self.media.retain(p, value) {
            state.views.insert(id.name.clone(), (id.clone(), body));
        }
    }
    fn apply(&self, mut event: ProviderEvent) {
        let provider = match &event {
            ProviderEvent::Message { provider, .. }
            | ProviderEvent::Directory { provider, .. }
            | ProviderEvent::Status { provider, .. }
            | ProviderEvent::Exited { provider, .. } => provider.clone(),
        };
        let Some(p) = self
            .providers
            .get(&provider.name)
            .filter(|p| p.id() == &provider)
        else {
            return;
        };
        let Ok(body) = self.media.event(p, &mut event) else {
            return;
        };
        let bytes = body.json.to_string().len();
        let mut state = self.state.lock().unwrap();
        match &event {
            ProviderEvent::Directory { update, .. } => {
                if let Err(error) = state.directory.apply(&provider, update.clone()) {
                    tracing::warn!(%error,"invalid directory update");
                    return;
                }
            }
            ProviderEvent::Status { status, .. } => {
                state.statuses.insert(provider.name.clone(), status.clone());
            }
            ProviderEvent::Exited { .. } => {
                state.directory.remove_provider(&provider);
                state.statuses.insert(
                    provider.name.clone(),
                    qmsg_types::ProviderStatus::Failed("provider exited".into()),
                );
            }
            _ => {}
        }
        self.refresh(p, &mut state);
        state.sequence += 1;
        let update = Arc::new(Update {
            sequence: state.sequence,
            provider,
            body,
            bytes,
        });
        state.bytes += bytes;
        state.ring.push_back(update.clone());
        while state.ring.len() > 2048 || state.bytes > 64 * 1024 * 1024 {
            if let Some(old) = state.ring.pop_front() {
                state.bytes -= old.bytes;
            }
        }
        let _ = self.updates.send(update);
    }
    pub fn provider(&self, id: Option<&wire::Provider>) -> Result<Arc<ProviderHandle>, Status> {
        let id = id.ok_or_else(|| Status::invalid_argument("provider required"))?;
        self.providers
            .get(&id.name)
            .filter(|p| p.id().instance == id.instance)
            .cloned()
            .ok_or_else(|| {
                Status::failed_precondition("provider instance changed; refresh snapshot")
            })
    }
    fn snapshot_locked(&self, device: &str, state: &State) -> Result<wire::Event, Status> {
        let mut providers = Vec::new();
        for (id, view) in state.views.values() {
            providers.push(
                serde_json::from_str::<serde_json::Value>(
                    &self.media.encode(device, id, view)?.json,
                )
                .unwrap(),
            );
        }
        Ok(wire::Event{epoch:self.epoch.clone(),sequence:state.sequence,body:Some(wire::event::Body::Snapshot(wire::Data{json:serde_json::json!({"providers":providers,"epoch":self.epoch,"sequence":state.sequence}).to_string(),blob_expires_at_ms:auth::now()+super::media::LIFETIME}))})
    }
    pub fn snapshot(&self, device: &str) -> Result<wire::Event, Status> {
        self.snapshot_locked(device, &self.state.lock().unwrap())
    }
    pub fn watch(
        self: &Arc<Self>,
        device: String,
        request: wire::WatchRequest,
        auth: Arc<auth::Auth>,
        token: String,
    ) -> Result<
        impl futures_util::Stream<Item = Result<wire::Event, Status>> + Send + 'static,
        Status,
    > {
        let state = self.state.lock().unwrap();
        let mut receiver = self.updates.subscribe();
        let first_retained = state
            .ring
            .front()
            .map_or(state.sequence + 1, |u| u.sequence);
        let initial: Vec<wire::Event> = if request.epoch != self.epoch
            || request.after_sequence > state.sequence
            || request.after_sequence.saturating_add(1) < first_retained
        {
            vec![self.snapshot_locked(&device, &state)?]
        } else {
            state
                .ring
                .iter()
                .filter(|u| u.sequence > request.after_sequence)
                .map(|u| self.event(&device, u))
                .collect::<Result<_, _>>()?
        };
        drop(state);
        let hub = self.clone();
        Ok(
            async_stream::try_stream! {let mut delivered=request.after_sequence;for event in initial {auth.check(&token)?;delivered=event.sequence;yield event;}let mut heartbeat=tokio::time::interval(std::time::Duration::from_secs(15));loop{
            let update=tokio::select!{update=receiver.recv()=>Some(update),_=heartbeat.tick()=>None};
            auth.check(&token)?;
            let event=if let Some(update)=update {let update=update.map_err(|_|Status::aborted("event stream fell behind; reconnect with cursor"))?;hub.event(&device,&update)?}else{let sequence=delivered;wire::Event{epoch:hub.epoch.clone(),sequence,body:Some(wire::event::Body::Heartbeat(true))}};
            delivered=event.sequence;yield event;
            }},
        )
    }
    fn event(&self, device: &str, update: &Update) -> Result<wire::Event, Status> {
        Ok(wire::Event {
            epoch: self.epoch.clone(),
            sequence: update.sequence,
            body: Some(wire::event::Body::Update(self.media.encode(
                device,
                &update.provider,
                &update.body,
            )?)),
        })
    }
    pub async fn barrier(&self) -> u64 {
        let (tx, rx) = oneshot::channel();
        if self.barriers.send(tx).await.is_err() {
            return self.state.lock().unwrap().sequence;
        }
        rx.await.unwrap_or_default()
    }
}
