//! Typed HTTP access to the server with transparent token refresh.

use crate::error::{ClientError, Result};
use crate::secrets::SecretStore;
use proto::*;
use reqwest::{Method, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, RwLock};

/// Persisted sign-in state (stored in the secret store, X-02).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub server: String,
    pub username: String,
    pub user_id: String,
    pub session_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    /// Pinned certificate fingerprint (TOFU) for self-signed servers.
    #[serde(default)]
    pub pin: Option<String>,
}

pub const SESSION_SECRET: &str = "atomic-chat.session";

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Turns what the user typed into a base URL. TLS is required: `http://` is only accepted when
/// `allow_insecure_http` is set (test servers on a LAN / emulator).
pub fn normalize_server_url(input: &str, allow_insecure_http: bool) -> Result<String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(ClientError::Local("enter the server address".into()));
    }
    let with_scheme = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let u = url::Url::parse(&with_scheme).map_err(|_| ClientError::Local("that server address is not valid".into()))?;
    match u.scheme() {
        "https" => {}
        "http" if allow_insecure_http => {}
        "http" => {
            return Err(ClientError::Local(
                "this server address uses http. Atomic Chat requires TLS (https) unless you explicitly allow an unencrypted test server.".into(),
            ))
        }
        _ => return Err(ClientError::Local("the server address must start with https://".into())),
    }
    let host = u.host_str().ok_or_else(|| ClientError::Local("that server address is not valid".into()))?;
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    Ok(format!("{}://{}{}", u.scheme(), host, port))
}

pub type FatalHandler = Arc<dyn Fn(ErrorCode, String) + Send + Sync>;

pub struct Api {
    pub base: String,
    http: reqwest::Client,
    session: RwLock<Option<Session>>,
    secrets: Arc<dyn SecretStore>,
    refresh_lock: tokio::sync::Mutex<()>,
    fatal: Mutex<Option<FatalHandler>>,
    version: String,
    pub pin: Option<String>,
}

impl Api {
    pub fn new(base: &str, pin: Option<String>, secrets: Arc<dyn SecretStore>, version: &str) -> Result<Api> {
        let cfg = crate::tls::client_config(pin.as_deref());
        let http = reqwest::Client::builder()
            .use_preconfigured_tls((*cfg).clone())
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(format!("atomic-chat/{version}"))
            .build()
            .map_err(|e| ClientError::Local(format!("http client: {e}")))?;
        Ok(Api {
            base: base.trim_end_matches('/').to_string(),
            http,
            session: RwLock::new(None),
            secrets,
            refresh_lock: tokio::sync::Mutex::new(()),
            fatal: Mutex::new(None),
            version: version.to_string(),
            pin,
        })
    }

    pub fn set_fatal_handler(&self, h: FatalHandler) {
        *self.fatal.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
    }

    pub fn session(&self) -> Option<Session> {
        self.session.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_session(&self, s: Option<Session>) {
        *self.session.write().unwrap_or_else(|e| e.into_inner()) = s;
    }

    pub fn persist(&self) -> Result<()> {
        match self.session() {
            Some(s) => self.secrets.set(SESSION_SECRET, &serde_json::to_string(&s)?),
            None => self.secrets.delete(SESSION_SECRET),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}{}", self.base, API_PREFIX, path)
    }

    pub fn ws_url(&self) -> String {
        let b = if let Some(r) = self.base.strip_prefix("https://") { format!("wss://{r}") } else { self.base.replacen("http://", "ws://", 1) };
        format!("{b}{API_PREFIX}/ws")
    }

    pub fn download_url(&self, attachment_id: &str) -> String {
        self.url(&format!("/files/{attachment_id}"))
    }

    fn net(e: reqwest::Error) -> ClientError {
        let mut msg = e.to_string();
        let mut src = std::error::Error::source(&e);
        while let Some(s) = src {
            msg = format!("{msg}: {s}");
            src = s.source();
        }
        ClientError::Network(msg)
    }

    async fn parse_error(&self, resp: Response) -> ClientError {
        let status = resp.status();
        match resp.json::<ErrorBody>().await {
            Ok(b) => {
                let e = b.error;
                if e.code.requires_wipe() {
                    if let Some(h) = self.fatal.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                        h(e.code, e.message.clone());
                    }
                }
                ClientError::Api { code: e.code, message: e.message }
            }
            Err(_) => ClientError::Api {
                code: if status.is_server_error() { ErrorCode::Internal } else { ErrorCode::Validation },
                message: format!("unexpected response (HTTP {})", status.as_u16()),
            },
        }
    }

    /// Refreshes the access token when it is missing or about to expire.
    pub async fn ensure_fresh(&self) -> Result<()> {
        let needs = self.session().map(|s| s.expires_at_ms - now_ms() < 30_000).unwrap_or(false);
        if needs {
            self.refresh().await?;
        }
        Ok(())
    }

    pub async fn refresh(&self) -> Result<()> {
        let before = self.session().ok_or(ClientError::NotSignedIn)?;
        let _g = self.refresh_lock.lock().await;
        // Another task may have refreshed while we waited.
        if let Some(cur) = self.session() {
            if cur.access_token != before.access_token {
                return Ok(());
            }
        }
        let resp = self
            .http
            .post(self.url("/refresh"))
            .header("X-Client-Version", &self.version)
            .json(&RefreshRequest { refresh_token: before.refresh_token.clone() })
            .send()
            .await
            .map_err(Self::net)?;
        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }
        let t: TokenPair = resp.json().await.map_err(Self::net)?;
        let mut s = before;
        s.access_token = t.access_token;
        s.expires_at_ms = now_ms() + t.expires_in * 1000;
        self.set_session(Some(s));
        self.persist()?;
        Ok(())
    }

    /// Sends a request built by `build`, retrying once after a token refresh.
    pub async fn exec(&self, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> Result<Response> {
        let mut attempt = 0;
        loop {
            self.ensure_fresh().await?;
            let mut rb = build(&self.http).header("X-Client-Version", &self.version);
            if let Some(s) = self.session() {
                rb = rb.bearer_auth(s.access_token);
            }
            let resp = rb.send().await.map_err(Self::net)?;
            if resp.status().is_success() {
                return Ok(resp);
            }
            let err = self.parse_error(resp).await;
            if attempt == 0 && matches!(err.code(), Some(ErrorCode::TokenExpired) | Some(ErrorCode::Unauthorized)) && self.session().is_some() {
                attempt += 1;
                self.refresh().await?;
                continue;
            }
            return Err(err);
        }
    }

    pub async fn json<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<serde_json::Value>) -> Result<T> {
        let url = self.url(path);
        let resp = self
            .exec(|c| {
                let r = c.request(method.clone(), &url);
                match &body {
                    Some(b) => r.json(b),
                    None => r,
                }
            })
            .await?;
        resp.json::<T>().await.map_err(Self::net)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.json(Method::GET, path, None).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        self.json(Method::POST, path, Some(serde_json::to_value(body)?)).await
    }

    pub async fn patch<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        self.json(Method::PATCH, path, Some(serde_json::to_value(body)?)).await
    }

    pub async fn put_json<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        self.json(Method::PUT, path, Some(serde_json::to_value(body)?)).await
    }

    pub async fn put<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.json(Method::PUT, path, None).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.json(Method::DELETE, path, None).await
    }

    // ----------------------------------------------------- unauthenticated

    pub async fn meta(&self) -> Result<ServerMeta> {
        let resp = self
            .http
            .get(self.url("/meta"))
            .header("X-Client-Version", &self.version)
            .send()
            .await
            .map_err(Self::net)?;
        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }
        resp.json().await.map_err(Self::net)
    }

    async fn login_post<B: Serialize>(&self, path: &str, body: &B) -> Result<LoginResponse> {
        let resp = self
            .http
            .post(self.url(path))
            .header("X-Client-Version", &self.version)
            .json(body)
            .send()
            .await
            .map_err(Self::net)?;
        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }
        resp.json().await.map_err(Self::net)
    }

    pub async fn login(&self, req: &LoginRequest) -> Result<LoginResponse> {
        self.login_post("/login", req).await
    }

    pub async fn login_link(&self, req: &LinkLoginRequest) -> Result<LoginResponse> {
        self.login_post("/login/link", req).await
    }

    pub async fn setup(&self, req: &SetupRequest) -> Result<LoginResponse> {
        self.login_post("/setup", req).await
    }

    /// Streams a file to `POST /files`. Not retried (the body is consumed); the token is
    /// refreshed beforehand.
    pub async fn upload(&self, name: &str, content_type: &str, len: u64, purpose: Option<&str>, body: reqwest::Body) -> Result<UploadResponse> {
        self.ensure_fresh().await?;
        let mut url = url::Url::parse(&self.url("/files")).map_err(|e| ClientError::Local(e.to_string()))?;
        url.query_pairs_mut().append_pair("name", name);
        if let Some(p) = purpose {
            url.query_pairs_mut().append_pair("purpose", p);
        }
        let mut rb = self
            .http
            .post(url)
            .header("X-Client-Version", &self.version)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .header(reqwest::header::CONTENT_LENGTH, len)
            .timeout(std::time::Duration::from_secs(3600))
            .body(body);
        if let Some(s) = self.session() {
            rb = rb.bearer_auth(s.access_token);
        }
        let resp = rb.send().await.map_err(Self::net)?;
        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }
        resp.json().await.map_err(Self::net)
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn current_access_token(&self) -> Option<String> {
        self.session().map(|s| s.access_token)
    }
}
