use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ClashIpcClient {
    config: ClashIpcClientConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClashIpcClientConfig {
    pub pipe_path: String,
    pub auth_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    #[serde(default)]
    pub premium: bool,
    #[serde(default)]
    pub meta: bool,
    #[serde(default)]
    pub commit: Option<String>,
}

impl ClashIpcClient {
    pub fn new(config: ClashIpcClientConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ClashIpcClientConfig {
        &self.config
    }

    pub async fn get_version(&self) -> anyhow::Result<VersionInfo> {
        self.get_json(ControllerEndpoint::Version.path()).await
    }

    pub async fn get_configs(&self) -> anyhow::Result<ControllerConfigs> {
        self.get_json(ControllerEndpoint::Configs.path()).await
    }

    pub async fn patch_configs(&self, configs: &ControllerConfigPatch) -> anyhow::Result<Value> {
        self.patch_json(ControllerEndpoint::Configs.path(), configs)
            .await
    }

    pub async fn reload_configs(&self, request: &ConfigReloadRequest) -> anyhow::Result<Value> {
        self.put_json(ControllerEndpoint::Configs.path(), request)
            .await
    }

    pub async fn get_proxies(&self) -> anyhow::Result<ProxyCollection> {
        self.get_json(ControllerEndpoint::Proxies.path()).await
    }

    pub async fn select_proxy(&self, group: &str, name: &str) -> anyhow::Result<Value> {
        self.put_json(
            &format!("/proxies/{}", encode_path_segment(group)),
            &ProxySelectionRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    pub async fn test_proxy_delay(
        &self,
        name: &str,
        url: &str,
        timeout_ms: u64,
    ) -> anyhow::Result<Value> {
        self.get_json(&format!(
            "/proxies/{}/delay?url={}&timeout={timeout_ms}",
            encode_path_segment(name),
            encode_query_component(url)
        ))
        .await
    }

    pub async fn get_group(&self, name: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/group/{}", encode_path_segment(name)))
            .await
    }

    pub async fn test_group_delay(
        &self,
        name: &str,
        url: &str,
        timeout_ms: u64,
    ) -> anyhow::Result<Value> {
        self.get_json(&format!(
            "/group/{}/delay?url={}&timeout={timeout_ms}",
            encode_path_segment(name),
            encode_query_component(url)
        ))
        .await
    }

    pub async fn get_proxy_providers(&self) -> anyhow::Result<ProviderCollection> {
        self.get_json(ControllerEndpoint::ProxyProviders.path())
            .await
    }

    pub async fn update_proxy_provider(&self, name: &str) -> anyhow::Result<Value> {
        self.put_empty(&format!("/providers/proxies/{}", encode_path_segment(name)))
            .await
    }

    pub async fn healthcheck_proxy_provider(&self, name: &str) -> anyhow::Result<Value> {
        self.put_empty(&format!(
            "/providers/proxies/{}/healthcheck",
            encode_path_segment(name)
        ))
        .await
    }

    pub async fn healthcheck_provider_proxy(
        &self,
        provider: &str,
        proxy: &str,
    ) -> anyhow::Result<Value> {
        self.get_json(&format!(
            "/providers/proxies/{}/{}/healthcheck",
            encode_path_segment(provider),
            encode_path_segment(proxy)
        ))
        .await
    }

    pub async fn get_rule_providers(&self) -> anyhow::Result<ProviderCollection> {
        self.get_json(ControllerEndpoint::RuleProviders.path())
            .await
    }

    pub async fn update_rule_provider(&self, name: &str) -> anyhow::Result<Value> {
        self.put_empty(&format!("/providers/rules/{}", encode_path_segment(name)))
            .await
    }

    pub async fn get_rule_provider_rules(&self, name: &str) -> anyhow::Result<Value> {
        self.get_json(&format!(
            "/providers/rules/{}/rules",
            encode_path_segment(name)
        ))
        .await
    }

    pub async fn test_rule_provider_match(
        &self,
        name: &str,
        target: &str,
    ) -> anyhow::Result<Value> {
        self.get_json(&format!(
            "/providers/rules/{}/match?target={}",
            encode_path_segment(name),
            encode_query_component(target)
        ))
        .await
    }

    pub async fn get_rules(&self) -> anyhow::Result<RulesResponse> {
        self.get_json(ControllerEndpoint::Rules.path()).await
    }

    pub async fn get_connections(&self) -> anyhow::Result<ConnectionsResponse> {
        self.get_json(ControllerEndpoint::Connections.path()).await
    }

    pub async fn close_all_connections(&self) -> anyhow::Result<Value> {
        self.delete_json(ControllerEndpoint::Connections.path())
            .await
    }

    pub async fn close_connection(&self, id: &str) -> anyhow::Result<Value> {
        self.delete_json(&format!("/connections/{}", encode_path_segment(id)))
            .await
    }

    pub async fn get_memory(&self) -> anyhow::Result<MemoryResponse> {
        self.get_json(ControllerEndpoint::Memory.path()).await
    }

    pub async fn query_dns(&self, name: &str, record_type: &str) -> anyhow::Result<Value> {
        self.get_json(&format!(
            "/dns/query?name={}&type={}",
            encode_query_component(name),
            encode_query_component(record_type)
        ))
        .await
    }

    pub async fn sample_websocket_text(
        &self,
        path: &str,
        max_messages: usize,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Vec<String>> {
        platform::websocket_text_messages(&self.config, path, max_messages, timeout).await
    }

    /// Open a persistent subscription to a controller WebSocket, invoking
    /// `on_message` for every text frame until `stop` is signalled or the
    /// socket closes. The subscription counterpart to [`Self::sample_websocket_text`].
    pub async fn stream_websocket_text(
        &self,
        path: &str,
        on_message: impl FnMut(String),
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        platform::stream_websocket_text(&self.config, path, on_message, stop).await
    }

    pub async fn get_json<T>(&self, path: &str) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        platform::request_json(&self.config, "GET", path, None).await
    }

    pub async fn post_empty<T>(&self, path: &str) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        platform::request_json(&self.config, "POST", path, None).await
    }

    pub async fn put_json<B, T>(&self, path: &str, body: &B) -> anyhow::Result<T>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let body = serde_json::to_vec(body)?;
        platform::request_json(&self.config, "PUT", path, Some(body)).await
    }

    pub async fn put_empty<T>(&self, path: &str) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        platform::request_json(&self.config, "PUT", path, None).await
    }

    pub async fn patch_json<B, T>(&self, path: &str, body: &B) -> anyhow::Result<T>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let body = serde_json::to_vec(body)?;
        platform::request_json(&self.config, "PATCH", path, Some(body)).await
    }

    pub async fn delete_json<T>(&self, path: &str) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        platform::request_json(&self.config, "DELETE", path, None).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfigs {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default, rename = "socks-port")]
    pub socks_port: Option<u16>,
    #[serde(default, rename = "mixed-port")]
    pub mixed_port: Option<u16>,
    #[serde(default, rename = "redir-port")]
    pub redir_port: Option<u16>,
    #[serde(default, rename = "tproxy-port")]
    pub tproxy_port: Option<u16>,
    #[serde(default, rename = "allow-lan")]
    pub allow_lan: Option<bool>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default, rename = "log-level")]
    pub log_level: Option<String>,
    #[serde(default)]
    pub ipv6: Option<bool>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControllerConfigPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "socks-port")]
    pub socks_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "mixed-port")]
    pub mixed_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "redir-port")]
    pub redir_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "tproxy-port")]
    pub tproxy_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "bind-address")]
    pub bind_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "log-level")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "allow-lan")]
    pub allow_lan: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigReloadRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySelectionRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyCollection {
    #[serde(default)]
    pub proxies: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCollection {
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesResponse {
    #[serde(default)]
    pub rules: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionsResponse {
    #[serde(default)]
    pub connections: Vec<Value>,
    #[serde(default, rename = "downloadTotal")]
    pub download_total: Option<u64>,
    #[serde(default, rename = "uploadTotal")]
    pub upload_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResponse {
    #[serde(default)]
    pub inuse: Option<u64>,
    #[serde(default)]
    pub oslimit: Option<u64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerEndpoint {
    Version,
    Configs,
    Proxies,
    ProxyProviders,
    RuleProviders,
    Rules,
    Connections,
    Logs,
    Traffic,
    Flows,
    DnsQuery,
    Memory,
    Restart,
}

fn encode_path_segment(value: &str) -> String {
    percent_encode(value, false)
}

fn encode_query_component(value: &str) -> String {
    percent_encode(value, true)
}

fn percent_encode(value: &str, space_as_plus: bool) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' if space_as_plus => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

mod platform {
    use anyhow::Context as _;
    use bytes::Bytes;
    use futures_util::StreamExt as _;
    use http_body_util::{BodyExt, Empty, Full};
    use hyper::{Request, client::conn::http1, header};
    use hyper_util::rt::TokioIo;
    use serde::de::DeserializeOwned;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    #[cfg(windows)]
    type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;
    #[cfg(unix)]
    type IpcStream = tokio::net::UnixStream;

    /// Windows named pipe or unix domain socket, by platform.
    async fn open_ipc(path: &str) -> anyhow::Result<IpcStream> {
        #[cfg(windows)]
        let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(path);
        #[cfg(unix)]
        let stream = tokio::net::UnixStream::connect(path).await;
        stream.with_context(|| format!("failed to open Clash controller endpoint {path}"))
    }

    use super::ClashIpcClientConfig;

    pub async fn request_json<T>(
        config: &ClashIpcClientConfig,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        if let Some(base_url) = remote_http_base(config) {
            let client = reqwest::Client::new();
            let method =
                reqwest::Method::from_bytes(method.as_bytes()).context("invalid HTTP method")?;
            let mut request = client.request(method, join_http_url(&base_url, path));
            if let Some(secret) = &config.auth_secret {
                request = request.bearer_auth(secret);
            }
            if let Some(body) = body {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body);
            }

            let response = request
                .send()
                .await
                .context("failed to send Clash controller request")?;
            let status = response.status();
            let bytes = response
                .bytes()
                .await
                .context("failed to read Clash controller response")?;

            if !status.is_success() {
                let text = String::from_utf8_lossy(&bytes);
                anyhow::bail!("Clash controller request failed with {status}: {text}");
            }

            if bytes.is_empty() {
                return Ok(serde_json::from_str("null")?);
            }

            return Ok(serde_json::from_slice(&bytes)?);
        }

        let client = open_ipc(&config.pipe_path).await?;
        let io = TokioIo::new(client);
        let (mut sender, connection) = http1::handshake(io)
            .await
            .context("failed to create HTTP/1 connection over Clash pipe")?;

        tokio::spawn(async move {
            let _ = connection.await;
        });

        let uri = format!("http://localhost{path}");
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost");

        if let Some(secret) = &config.auth_secret {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }

        let request = if let Some(body) = body {
            builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(body)).boxed())?
        } else {
            builder.body(Empty::<Bytes>::new().boxed())?
        };

        let response = sender
            .send_request(request)
            .await
            .context("failed to send Clash controller request")?;

        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .context("failed to read Clash controller response")?
            .to_bytes();

        if !status.is_success() {
            let text = String::from_utf8_lossy(&bytes);
            anyhow::bail!("Clash controller request failed with {status}: {text}");
        }

        if bytes.is_empty() {
            return Ok(serde_json::from_str("null")?);
        }

        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn websocket_text_messages(
        config: &ClashIpcClientConfig,
        path: &str,
        max_messages: usize,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Vec<String>> {
        if let Some(base_url) = remote_ws_base(config) {
            let mut websocket = open_remote_websocket(&base_url, path, config).await?;
            return collect_websocket_messages(&mut websocket, max_messages, timeout).await;
        }

        let mut websocket = open_pipe_websocket(path, config).await?;
        collect_websocket_messages(&mut websocket, max_messages, timeout).await
    }

    /// Open the controller WebSocket and invoke `on_message` for every text
    /// frame until `stop` flips to `true` or the socket closes. Unlike
    /// [`websocket_text_messages`], this keeps the connection open and delivers
    /// frames as they arrive — the persistent-subscription counterpart to the
    /// batch sampler. Returns once the stream ends or a stop is signalled.
    pub async fn stream_websocket_text(
        config: &ClashIpcClientConfig,
        path: &str,
        on_message: impl FnMut(String),
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        if let Some(base_url) = remote_ws_base(config) {
            let mut websocket = open_remote_websocket(&base_url, path, config).await?;
            return drive_websocket_stream(&mut websocket, on_message, stop).await;
        }

        let mut websocket = open_pipe_websocket(path, config).await?;
        drive_websocket_stream(&mut websocket, on_message, stop).await
    }

    async fn open_remote_websocket(
        base_url: &str,
        path: &str,
        config: &ClashIpcClientConfig,
    ) -> anyhow::Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    > {
        let mut request =
            format!("{}{path}", base_url.trim_end_matches('/')).into_client_request()?;
        if let Some(secret) = &config.auth_secret {
            request
                .headers_mut()
                .insert("authorization", format!("Bearer {secret}").parse()?);
        }
        let (websocket, _) = tokio_tungstenite::connect_async(request)
            .await
            .context("failed to open remote Clash controller WebSocket")?;
        Ok(websocket)
    }

    async fn open_pipe_websocket(
        path: &str,
        config: &ClashIpcClientConfig,
    ) -> anyhow::Result<tokio_tungstenite::WebSocketStream<IpcStream>> {
        let client = open_ipc(&config.pipe_path).await?;

        let mut request = format!("ws://localhost{path}").into_client_request()?;
        if let Some(secret) = &config.auth_secret {
            request
                .headers_mut()
                .insert("authorization", format!("Bearer {secret}").parse()?);
        }
        let (websocket, _) = tokio_tungstenite::client_async(request, client)
            .await
            .context("failed to open Clash controller WebSocket")?;
        Ok(websocket)
    }

    async fn drive_websocket_stream<S>(
        websocket: &mut tokio_tungstenite::WebSocketStream<S>,
        mut on_message: impl FnMut(String),
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        if *stop.borrow() {
            return Ok(());
        }
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    // Sender dropped or value flipped to true — either way, stop.
                    if changed.is_err() || *stop.borrow() {
                        return Ok(());
                    }
                }
                message = websocket.next() => {
                    let Some(message) = message else {
                        return Ok(());
                    };
                    match message? {
                        Message::Text(text) => on_message(text.to_string()),
                        Message::Binary(bytes) => {
                            on_message(String::from_utf8_lossy(&bytes).into_owned())
                        }
                        Message::Close(_) => return Ok(()),
                        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                    }
                }
            }
        }
    }

    async fn collect_websocket_messages<S>(
        websocket: &mut tokio_tungstenite::WebSocketStream<S>,
        max_messages: usize,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Vec<String>>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut messages = Vec::new();
        while messages.len() < max_messages {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            let Some(message) = tokio::time::timeout(remaining, websocket.next()).await? else {
                break;
            };
            match message? {
                Message::Text(text) => messages.push(text.to_string()),
                Message::Binary(bytes) => messages.push(String::from_utf8_lossy(&bytes).into_owned()),
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }

        Ok(messages)
    }

    fn remote_http_base(config: &ClashIpcClientConfig) -> Option<String> {
        let endpoint = config.pipe_path.trim();
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            Some(endpoint.trim_end_matches('/').to_owned())
        } else {
            None
        }
    }

    fn remote_ws_base(config: &ClashIpcClientConfig) -> Option<String> {
        let base = remote_http_base(config)?;
        if let Some(stripped) = base.strip_prefix("http://") {
            return Some(format!("ws://{stripped}"));
        }
        if let Some(stripped) = base.strip_prefix("https://") {
            return Some(format!("wss://{stripped}"));
        }
        None
    }

    fn join_http_url(base: &str, path: &str) -> String {
        format!("{}{path}", base.trim_end_matches('/'))
    }
}

impl ControllerEndpoint {
    pub fn path(self) -> &'static str {
        match self {
            Self::Version => "/version",
            Self::Configs => "/configs",
            Self::Proxies => "/proxies",
            Self::ProxyProviders => "/providers/proxies",
            Self::RuleProviders => "/providers/rules",
            Self::Rules => "/rules",
            Self::Connections => "/connections",
            Self::Logs => "/ws/logs",
            Self::Traffic => "/ws/traffic",
            Self::Flows => "/ws/flows",
            Self::DnsQuery => "/dns/query",
            Self::Memory => "/memory",
            Self::Restart => "/restart",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerEndpoint, VersionInfo};

    #[test]
    fn endpoint_paths_match_dashboard_api() {
        assert_eq!(ControllerEndpoint::Version.path(), "/version");
        assert_eq!(ControllerEndpoint::Configs.path(), "/configs");
        assert_eq!(ControllerEndpoint::Traffic.path(), "/ws/traffic");
        assert_eq!(ControllerEndpoint::Flows.path(), "/ws/flows");
    }

    #[test]
    fn version_info_accepts_master_commit_field() {
        let version: VersionInfo = serde_json::from_str(
            r#"{"version":"0.10.6-alpha+sha.abc1234","meta":false,"commit":"abc1234"}"#,
        )
        .unwrap();

        assert_eq!(version.version, "0.10.6-alpha+sha.abc1234");
        assert_eq!(version.commit.as_deref(), Some("abc1234"));
        assert!(!version.meta);
    }

    #[test]
    fn connections_response_reads_camelcase_totals() {
        // The Clash controller emits camelCase `uploadTotal`/`downloadTotal`.
        // Without the serde rename these silently deserialize to None and the
        // UI's cumulative traffic counters stay blank.
        let response: super::ConnectionsResponse =
            serde_json::from_str(r#"{"connections":[],"uploadTotal":4096,"downloadTotal":1024}"#)
                .unwrap();

        assert_eq!(response.upload_total, Some(4096));
        assert_eq!(response.download_total, Some(1024));
    }

    /// A persistent subscription to an unreachable endpoint must fail fast and
    /// return — never hang — so the UI's effect-cleanup teardown can't deadlock.
    #[cfg(windows)]
    #[tokio::test]
    async fn stream_websocket_returns_on_unreachable_endpoint() {
        use super::{ClashIpcClient, ClashIpcClientConfig};

        let client = ClashIpcClient::new(ClashIpcClientConfig {
            pipe_path: r"\\.\pipe\clash-rs-desktop-test-nonexistent".to_owned(),
            auth_secret: None,
        });
        let (_stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.stream_websocket_text("/ws/traffic", |_frame| {}, stop_rx),
        )
        .await;

        // The open fails (no such pipe); the call must resolve, not time out.
        assert!(
            result.is_ok(),
            "stream_websocket_text hung on a dead endpoint"
        );
        assert!(result.unwrap().is_err(), "expected a connection error");
    }

    /// A pre-signalled stop must short-circuit before any frame is delivered.
    #[cfg(windows)]
    #[tokio::test]
    async fn stream_websocket_honors_prestopped_signal() {
        use super::{ClashIpcClient, ClashIpcClientConfig};

        let client = ClashIpcClient::new(ClashIpcClientConfig {
            pipe_path: r"\\.\pipe\clash-rs-desktop-test-nonexistent".to_owned(),
            auth_secret: None,
        });
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        stop_tx.send(true).unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.stream_websocket_text("/ws/traffic", |_frame| {}, stop_rx),
        )
        .await;

        assert!(
            result.is_ok(),
            "stream_websocket_text hung despite a pre-set stop"
        );
    }
}
