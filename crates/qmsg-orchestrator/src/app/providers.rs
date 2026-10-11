//! Paired clients manage accounts backed by server-installed components.
use super::{Service, hub};
use crate::{Orchestrator, ProviderHandle, ProviderSpec};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, mpsc};
use tonic::Status;

#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedProvider {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub enabled: bool,
    pub settings: BTreeMap<String, String>,
}
#[derive(Serialize, schemars::JsonSchema)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub required: bool,
    pub secret: bool,
}
#[derive(Serialize, schemars::JsonSchema)]
pub struct ProviderType {
    pub id: String,
    pub label: String,
    pub fields: Vec<Field>,
    pub defaults: BTreeMap<String, String>,
}
#[derive(Serialize, schemars::JsonSchema)]
pub struct Catalog {
    pub types: Vec<ProviderType>,
    pub accounts: Vec<ManagedProvider>,
}
struct State {
    accounts: BTreeMap<String, ManagedProvider>,
    active: BTreeMap<String, Arc<ProviderHandle>>,
}
pub(super) struct Manager {
    host: Arc<Orchestrator>,
    templates: BTreeMap<String, ProviderSpec>,
    events: mpsc::Sender<crate::ProviderEvent>,
    hub: Arc<hub::Hub>,
    path: PathBuf,
    state: Mutex<State>,
}
fn fields(spec: &ProviderSpec) -> Vec<Field> {
    let keys: Vec<String> = if spec.name == "echo" {
        vec!["server".into()]
    } else if spec.name == "gmail" {
        vec!["address".into()]
    } else {
        spec.settings
            .keys()
            .filter(|key| {
                !key.starts_with("test_")
                    && !["token", "password", "secret"]
                        .iter()
                        .any(|word| key.contains(word))
            })
            .cloned()
            .collect()
    };
    keys.into_iter()
        .map(|id| Field {
            label: if id == "address" {
                "Email address".into()
            } else if id == "server" {
                "Server address".into()
            } else {
                id.replace('_', " ")
            },
            required: id == "server" || id == "address",
            secret: false,
            id,
        })
        .collect()
}
fn public_settings(spec: &ProviderSpec) -> BTreeMap<String, String> {
    fields(spec)
        .into_iter()
        .filter_map(|field| {
            spec.settings
                .get(&field.id)
                .map(|value| (field.id, value.clone()))
        })
        .collect()
}
impl Manager {
    pub async fn service(
        data: &Path,
        host: Orchestrator,
        specs: Vec<ProviderSpec>,
    ) -> anyhow::Result<Service> {
        let path = data.join("providers.json");
        let templates: BTreeMap<_, _> = specs
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect();
        let accounts: Vec<ManagedProvider> = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)?
        } else {
            templates
                .values()
                .map(|spec| ManagedProvider {
                    name: spec.name.clone(),
                    label: spec.name.clone(),
                    kind: spec.name.clone(),
                    enabled: true,
                    settings: public_settings(spec),
                })
                .collect()
        };
        anyhow::ensure!(accounts.len() <= 32, "too many managed accounts");
        let host = Arc::new(host);
        let (events, receiver) = mpsc::channel(256);
        let mut state = State {
            accounts: BTreeMap::new(),
            active: BTreeMap::new(),
        };
        for account in accounts {
            let template = templates.get(&account.kind).ok_or_else(|| {
                anyhow::anyhow!("provider type is no longer installed: {}", account.kind)
            })?;
            anyhow::ensure!(
                !state.accounts.contains_key(&account.name),
                "duplicate account name"
            );
            if account.enabled {
                let mut spec = template.clone();
                spec.name = account.name.clone();
                spec.settings.extend(account.settings.clone());
                let mut handle = host.spawn(spec, events.clone())?;
                handle.set_timeout(None);
                state.active.insert(account.name.clone(), Arc::new(handle));
            }
            state.accounts.insert(account.name.clone(), account);
        }
        let mut service = Service::new(data, state.active.values().cloned().collect(), receiver)?;
        service.management = Some(Arc::new(Self {
            host,
            templates,
            events,
            hub: service.hub.clone(),
            path,
            state: Mutex::new(state),
        }));
        Ok(service)
    }
    pub async fn catalog(&self) -> Catalog {
        Catalog {
            types: self
                .templates
                .values()
                .map(|spec| ProviderType {
                    id: spec.name.clone(),
                    label: match spec.name.as_str() {
                        "discord" => "Discord".into(),
                        "echo" => "Echo".into(),
                        "email" => "Email".into(),
                        "gmail" => "Gmail".into(),
                        _ => spec.name.clone(),
                    },
                    fields: fields(spec),
                    defaults: public_settings(spec),
                })
                .collect(),
            accounts: self.state.lock().await.accounts.values().cloned().collect(),
        }
    }
    fn persist(&self, accounts: &BTreeMap<String, ManagedProvider>) -> Result<(), Status> {
        let values: Vec<_> = accounts.values().collect();
        let mut file = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())
            .map_err(|_| Status::internal("could not save accounts"))?;
        serde_json::to_writer_pretty(file.as_file_mut(), &values)
            .map_err(|_| Status::internal("could not save accounts"))?;
        file.as_file()
            .sync_all()
            .map_err(|_| Status::internal("could not save accounts"))?;
        file.persist(&self.path)
            .map_err(|_| Status::internal("could not save accounts"))?;
        Ok(())
    }
    async fn stop(&self, state: &mut State, name: &str) -> Result<(), Status> {
        if let Some(handle) = state.active.remove(name) {
            handle.kill();
        }
        self.hub.remove_provider(name);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while self.host.provider_running(name) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| Status::unavailable("account is still stopping; try again"))?;
        Ok(())
    }
    fn restore(&self, state: &mut State, account: &ManagedProvider) {
        if !account.enabled {
            return;
        }
        let Some(template) = self.templates.get(&account.kind) else {
            return;
        };
        let mut spec = template.clone();
        spec.name = account.name.clone();
        spec.settings.extend(account.settings.clone());
        if let Ok(handle) = self
            .hub
            .start_provider(|| self.host.spawn(spec, self.events.clone()))
        {
            state.active.insert(account.name.clone(), handle);
        }
    }
    pub async fn configure(&self, mut account: ManagedProvider) -> Result<(), Status> {
        if account.name.is_empty()
            || account.name.len() > 64
            || !account
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Status::invalid_argument("invalid account name"));
        }
        let template = self
            .templates
            .get(&account.kind)
            .ok_or_else(|| Status::invalid_argument("provider type is not installed"))?;
        let mut state = self.state.lock().await;
        let previous = state.accounts.get(&account.name).cloned();
        if account.label.is_empty() {
            account.label = previous
                .as_ref()
                .map_or_else(|| account.name.clone(), |old| old.label.clone());
        }
        if account.label.len() > 80 || account.label.chars().any(char::is_control) {
            return Err(Status::invalid_argument("invalid account label"));
        }
        let fields = fields(template);
        if account
            .settings
            .iter()
            .any(|(key, value)| value.len() > 8192 || !fields.iter().any(|field| &field.id == key))
        {
            return Err(Status::invalid_argument("unsupported account setting"));
        }
        if fields.iter().any(|field| {
            field.required
                && account
                    .settings
                    .get(&field.id)
                    .is_none_or(|value| value.trim().is_empty())
        }) {
            return Err(Status::invalid_argument(
                "a required account setting is missing",
            ));
        }
        if previous
            .as_ref()
            .is_some_and(|old| old.kind != account.kind)
        {
            return Err(Status::invalid_argument(
                "an account's provider type cannot change",
            ));
        }
        if previous.is_none() && state.accounts.len() >= 32 {
            return Err(Status::resource_exhausted("too many accounts"));
        }
        if previous.as_ref() == Some(&account)
            && (!account.enabled || state.active.contains_key(&account.name))
        {
            return Ok(());
        }
        let restart = previous
            .as_ref()
            .is_none_or(|old| old.enabled != account.enabled || old.settings != account.settings)
            || (account.enabled && !state.active.contains_key(&account.name));
        if restart {
            self.stop(&mut state, &account.name).await?;
        }
        if restart && account.enabled {
            let mut spec = template.clone();
            spec.name = account.name.clone();
            spec.settings.extend(account.settings.clone());
            let handle = match self
                .hub
                .start_provider(|| self.host.spawn(spec, self.events.clone()))
            {
                Ok(handle) => handle,
                Err(_) => {
                    if let Some(old) = &previous {
                        self.restore(&mut state, old);
                    }
                    return Err(Status::internal("could not start account"));
                }
            };
            state.active.insert(account.name.clone(), handle);
        }
        state.accounts.insert(account.name.clone(), account.clone());
        if let Err(error) = self.persist(&state.accounts) {
            // Roll back both the saved configuration and runtime if disk persistence failed.
            self.stop(&mut state, &account.name).await?;
            state.accounts.remove(&account.name);
            if let Some(previous) = previous {
                self.restore(&mut state, &previous);
                state.accounts.insert(previous.name.clone(), previous);
            }
            return Err(error);
        }
        Ok(())
    }
    pub async fn remove(&self, name: &str) -> Result<(), Status> {
        let mut state = self.state.lock().await;
        if !state.accounts.contains_key(name) {
            return Ok(());
        }
        let mut accounts = state.accounts.clone();
        accounts.remove(name);
        self.persist(&accounts)?;
        if let Err(error) = self.stop(&mut state, name).await {
            self.persist(&state.accounts)?;
            return Err(error);
        }
        if self.host.forget_provider(name).is_err() {
            self.persist(&state.accounts)?;
            if let Some(old) = state.accounts.get(name).cloned() {
                self.restore(&mut state, &old);
            }
            return Err(Status::internal("could not remove saved account session"));
        }
        state.accounts = accounts;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{wire, wire::app_server::App};
    use super::*;
    use crate::Encryption;
    use tonic::Request;

    #[test]
    fn gmail_setup_only_asks_for_the_mail_address() {
        let spec = ProviderSpec {
            name: "gmail".into(),
            wasm: "email.wasm".into(),
            settings: [
                ("service".into(), "gmail".into()),
                ("address".into(), "".into()),
                ("oauth_client_id".into(), "registered-client".into()),
                ("oauth_redirect_uri".into(), "registered-callback".into()),
            ]
            .into(),
        };
        let visible = fields(&spec);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "address");
        assert!(visible[0].required);
        assert_eq!(
            public_settings(&spec),
            [("address".into(), "".into())].into()
        );
    }

    #[tokio::test]
    async fn account_changes_reset_open_and_resumed_watch_streams() {
        use futures_util::StreamExt;
        let dir = tempfile::tempdir().unwrap();
        let account = ManagedProvider {
            name: "echo".into(),
            label: "Echo".into(),
            kind: "echo".into(),
            enabled: false,
            settings: [("server".into(), "127.0.0.1:1234".into())].into(),
        };
        std::fs::write(
            dir.path().join("providers.json"),
            serde_json::to_vec(&vec![account.clone()]).unwrap(),
        )
        .unwrap();
        let host = Orchestrator::new(dir.path(), Encryption::Plaintext)
            .await
            .unwrap();
        let spec = ProviderSpec {
            name: "echo".into(),
            wasm: "unused.wasm".into(),
            settings: account.settings.clone(),
        };
        let service = Manager::service(dir.path(), host, vec![spec])
            .await
            .unwrap();
        let (code, _) = service.auth.grant().unwrap();
        let credential = service.auth.pair(&code, "test").unwrap();
        let cursor = service.hub.snapshot(&credential.device_id).unwrap();
        let stream = service
            .hub
            .watch(
                credential.device_id.clone(),
                wire::WatchRequest {
                    epoch: cursor.epoch.clone(),
                    after_sequence: cursor.sequence,
                },
                service.auth.clone(),
                credential.token.clone(),
            )
            .unwrap();
        tokio::pin!(stream);
        let manager = service.management.as_ref().unwrap();
        let mut enabled = account.clone();
        enabled.enabled = true;
        manager.configure(enabled).await.unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let event = stream.next().await.unwrap().unwrap();
                if matches!(event.body, Some(wire::event::Body::Snapshot(_))) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        let Some(wire::event::Body::Snapshot(data)) = event.body else {
            unreachable!()
        };
        let value: serde_json::Value = serde_json::from_str(&data.json).unwrap();
        assert_eq!(value["providers"].as_array().unwrap().len(), 1);
        manager.configure(account).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let event = stream.next().await.unwrap().unwrap();
                if let Some(wire::event::Body::Snapshot(data)) = event.body {
                    let value: serde_json::Value = serde_json::from_str(&data.json).unwrap();
                    assert!(value["providers"].as_array().unwrap().is_empty());
                    break;
                }
            }
        })
        .await
        .unwrap();
        let resumed = service
            .hub
            .watch(
                credential.device_id,
                wire::WatchRequest {
                    epoch: cursor.epoch,
                    after_sequence: cursor.sequence,
                },
                service.auth.clone(),
                credential.token,
            )
            .unwrap();
        tokio::pin!(resumed);
        assert!(matches!(
            resumed.next().await.unwrap().unwrap().body,
            Some(wire::event::Body::Snapshot(_))
        ));
    }

    #[tokio::test]
    async fn accounts_persist_validate_and_require_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let initial = ManagedProvider {
            name: "echo".into(),
            label: "Echo".into(),
            kind: "echo".into(),
            enabled: false,
            settings: [("server".into(), "127.0.0.1:1234".into())].into(),
        };
        std::fs::write(
            dir.path().join("providers.json"),
            serde_json::to_vec(&vec![initial.clone()]).unwrap(),
        )
        .unwrap();
        let spec = ProviderSpec {
            name: "echo".into(),
            wasm: "unused.wasm".into(),
            settings: initial.settings.clone(),
        };
        let host = Orchestrator::new(dir.path(), Encryption::Plaintext)
            .await
            .unwrap();
        let service = Manager::service(dir.path(), host, vec![spec.clone()])
            .await
            .unwrap();
        assert_eq!(
            service
                .provider_catalog(Request::new(wire::Empty {}))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        assert_eq!(
            service
                .configure_provider(Request::new(wire::ProviderConfiguration::default()))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        assert_eq!(
            service
                .remove_provider(Request::new(wire::ProviderName::default()))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        let manager = service.management.as_ref().unwrap();
        manager
            .host
            .secrets
            .set("echo", "token", b"saved session")
            .unwrap();
        manager.configure(initial.clone()).await.unwrap();
        assert_eq!(
            manager.host.secrets.get("echo", "token").unwrap(),
            Some(b"saved session".to_vec())
        );
        let mut account = initial.clone();
        account.name = "echo-work".into();
        account.label = "Work".into();
        manager.configure(account.clone()).await.unwrap();
        account.label = "Renamed".into();
        manager.configure(account.clone()).await.unwrap();
        let mut invalid = account.clone();
        invalid
            .settings
            .insert("token".into(), "must not enter public settings".into());
        assert_eq!(
            manager.configure(invalid).await.unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        let mut invalid = account.clone();
        invalid.kind = "arbitrary-component".into();
        assert_eq!(
            manager.configure(invalid).await.unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        assert_eq!(manager.catalog().await.accounts.len(), 2);
        drop(service);
        let host = Orchestrator::new(dir.path(), Encryption::Plaintext)
            .await
            .unwrap();
        let service = Manager::service(dir.path(), host, vec![spec])
            .await
            .unwrap();
        let manager = service.management.as_ref().unwrap();
        assert_eq!(
            manager
                .catalog()
                .await
                .accounts
                .iter()
                .find(|a| a.name == account.name)
                .unwrap()
                .label,
            "Renamed"
        );
        manager.remove(&account.name).await.unwrap();
        assert_eq!(manager.catalog().await.accounts.len(), 1);
        assert!(
            manager
                .catalog()
                .await
                .accounts
                .iter()
                .all(|a| a.name != account.name)
        );
        let saved: Vec<ManagedProvider> =
            serde_json::from_slice(&std::fs::read(dir.path().join("providers.json")).unwrap())
                .unwrap();
        assert_eq!(saved.len(), 1);
        manager.remove("echo").await.unwrap();
        assert_eq!(manager.host.secrets.get("echo", "token").unwrap(), None);
    }
}
