use crate::{ChatGptAccount, CredentialStore, CredentialStoreError, account::ChatGptAccountData};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, IssuerUrl, OAuth2TokenResponse,
    PkceCodeChallenge, RedirectUrl, RefreshToken, Scope, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use std::{
    error::Error,
    fmt,
    io::{self, BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const CALLBACK_PATH: &str = "/auth/callback";
const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
const APP_NAME: &str = "Carapanã";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const REFRESH_WINDOW: Duration = Duration::from_secs(300);
const REQUESTED_SCOPES: [&str; 6] = [
    "openid",
    "profile",
    "email",
    "offline_access",
    "resource.invoke",
    "chatgpt.tokens.use.direct",
];

struct AuthorizationAttempt {
    authorization_url: openidconnect::url::Url,
    state: openidconnect::CsrfToken,
    nonce: openidconnect::Nonce,
    verifier: openidconnect::PkceCodeVerifier,
}

struct AuthorizationOptions<'a> {
    registered_client_id: Option<&'a str>,
    host_id: &'a str,
    id_token_hint: Option<&'a str>,
    login_hint: Option<&'a str>,
    app_name: &'a str,
    request_consent: bool,
}

struct AuthorizationCallback {
    code: String,
    client_id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    InvalidCallback,
    StateMismatch,
    AuthorizationDenied,
    AuthorizationFailed,
    InvalidClientId,
    ClientIdMismatch,
    AccountMismatch,
    ProviderUnavailable,
    InvalidProviderMetadata,
    CallbackUnavailable,
    CallbackTimeout,
    BrowserUnavailable,
    TokenExchangeFailed,
    InvalidTokenResponse,
    InvalidIdentity,
    CredentialStorage,
    AccountNotFound,
    RefreshRequired,
    RefreshFailed,
    RefreshRejected,
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidCallback => "the sign-in callback was invalid",
            Self::StateMismatch => "the sign-in state did not match",
            Self::AuthorizationDenied => "ChatGPT authorization was declined",
            Self::AuthorizationFailed => "ChatGPT authorization did not complete",
            Self::InvalidClientId => "ChatGPT did not return a valid app registration",
            Self::ClientIdMismatch => "ChatGPT returned a different app registration",
            Self::AccountMismatch => "the signed-in account does not match the selected profile",
            Self::ProviderUnavailable => "ChatGPT sign-in is temporarily unavailable",
            Self::InvalidProviderMetadata => "ChatGPT sign-in configuration could not be verified",
            Self::CallbackUnavailable => "the local sign-in callback could not be started",
            Self::CallbackTimeout => "ChatGPT sign-in timed out",
            Self::BrowserUnavailable => "the system browser could not be opened",
            Self::TokenExchangeFailed => "ChatGPT authorization could not be exchanged",
            Self::InvalidTokenResponse => "ChatGPT returned an invalid credential response",
            Self::InvalidIdentity => "the ChatGPT identity could not be verified",
            Self::CredentialStorage => "ChatGPT credentials could not be stored securely",
            Self::AccountNotFound => "the selected ChatGPT account was not found locally",
            Self::RefreshRequired => "ChatGPT sign-in needs to be renewed",
            Self::RefreshFailed => "ChatGPT credentials could not be refreshed",
            Self::RefreshRejected => "ChatGPT rejected the saved refresh credential",
        };
        formatter.write_str(message)
    }
}

impl Error for AuthError {}

impl From<CredentialStoreError> for AuthError {
    fn from(_: CredentialStoreError) -> Self {
        Self::CredentialStorage
    }
}

struct AuthorizationGrant {
    callback: AuthorizationCallback,
    attempt: AuthorizationAttempt,
    redirect_uri: RedirectUrl,
}

/// Perform the official SIWC authorization flow and persist the validated account.
/// Pass an issued client ID to reauthorize that exact local profile, or `None` to register.
pub fn authenticate(registered_client_id: Option<&str>) -> Result<ChatGptAccount, AuthError> {
    authenticate_with_options(registered_client_id, false)
}

/// Repeat OAuth with an explicit consent prompt after the operator chooses to enable
/// direct ChatGPT-plan usage for an already registered account.
pub fn enable_plan_usage(registered_client_id: &str) -> Result<ChatGptAccount, AuthError> {
    if !valid_client_id(registered_client_id) {
        return Err(AuthError::InvalidClientId);
    }
    authenticate_with_options(Some(registered_client_id), true)
}

fn authenticate_with_options(
    registered_client_id: Option<&str>,
    request_consent: bool,
) -> Result<ChatGptAccount, AuthError> {
    let store = CredentialStore::for_user()?;
    let _account_lock = match registered_client_id {
        Some(client_id) => Some(store.lock_account(client_id)?),
        None => None,
    };
    let previous = match registered_client_id {
        Some(client_id) => Some(store.load(client_id)?.ok_or(AuthError::AccountNotFound)?),
        None => None,
    };
    let host_id = store.host_id()?;
    let http_client = oidc_http_client()?;
    let metadata = discover_provider(&http_client)?;
    let authorization_options = AuthorizationOptions {
        registered_client_id,
        host_id: &host_id,
        id_token_hint: previous.as_ref().map(ChatGptAccount::id_token),
        login_hint: previous.as_ref().and_then(ChatGptAccount::email),
        app_name: APP_NAME,
        request_consent,
    };
    let grant = authorization_grant(&metadata, &authorization_options, open_system_browser)?;
    let account = exchange_authorization_code(
        &metadata,
        &http_client,
        grant,
        previous.as_ref().map(ChatGptAccount::subject),
    )?;
    store.save(&account)?;
    Ok(account)
}

/// Load a selected local account and rotate its refresh token before access-token expiry.
/// A process-wide file lock serializes rotation for the same SIWC registration.
pub fn load_fresh_account(client_id: &str) -> Result<ChatGptAccount, AuthError> {
    let store = CredentialStore::for_user()?;
    let _lock = store.lock_account(client_id)?;
    let account = store.load(client_id)?.ok_or(AuthError::AccountNotFound)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::RefreshFailed)?
        .as_secs();
    if account.expires_at_unix_seconds().saturating_sub(now) > REFRESH_WINDOW.as_secs() {
        return Ok(account);
    }
    let refresh_token = account.refresh_token().ok_or(AuthError::RefreshRequired)?;
    let http_client = oidc_http_client()?;
    let metadata = discover_provider(&http_client)?;
    let refreshed = match refresh_account(&metadata, &http_client, &account, refresh_token) {
        Ok(account) => account,
        Err(AuthError::RefreshRejected) => {
            store.save(&account.without_refresh_token())?;
            return Err(AuthError::RefreshRequired);
        }
        Err(error) => return Err(error),
    };
    store.save(&refreshed)?;
    Ok(refreshed)
}

fn oidc_http_client() -> Result<openidconnect::reqwest::blocking::Client, AuthError> {
    openidconnect::reqwest::blocking::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| AuthError::ProviderUnavailable)
}

fn discover_provider(
    http_client: &openidconnect::reqwest::blocking::Client,
) -> Result<CoreProviderMetadata, AuthError> {
    let issuer =
        IssuerUrl::new(ISSUER.to_owned()).map_err(|_| AuthError::InvalidProviderMetadata)?;
    let metadata = CoreProviderMetadata::discover(&issuer, http_client)
        .map_err(|_| AuthError::ProviderUnavailable)?;
    if metadata.issuer().as_str() != ISSUER
        || !official_auth_endpoint(metadata.authorization_endpoint().as_str())
        || !official_auth_endpoint(
            metadata
                .token_endpoint()
                .ok_or(AuthError::InvalidProviderMetadata)?
                .as_str(),
        )
        || !official_auth_endpoint(metadata.jwks_uri().as_str())
    {
        return Err(AuthError::InvalidProviderMetadata);
    }
    Ok(metadata)
}

fn official_auth_endpoint(value: &str) -> bool {
    let Ok(url) = openidconnect::url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("auth.openai.com")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn authorization_grant(
    metadata: &CoreProviderMetadata,
    options: &AuthorizationOptions<'_>,
    open_browser: impl FnOnce(&str) -> Result<(), AuthError>,
) -> Result<AuthorizationGrant, AuthError> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|_| AuthError::CallbackUnavailable)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| AuthError::CallbackUnavailable)?;
    let address = listener
        .local_addr()
        .map_err(|_| AuthError::CallbackUnavailable)?;
    let redirect_uri = RedirectUrl::new(format!(
        "http://127.0.0.1:{}{CALLBACK_PATH}",
        address.port()
    ))
    .map_err(|_| AuthError::CallbackUnavailable)?;
    if !valid_host_id(options.host_id)
        || options
            .registered_client_id
            .is_some_and(|client_id| !valid_client_id(client_id))
    {
        return Err(AuthError::InvalidClientId);
    }
    let attempt = authorization_attempt(metadata, redirect_uri.clone(), options);
    open_browser(attempt.authorization_url.as_str())?;
    let callback = wait_for_callback(
        &listener,
        address.port(),
        &attempt.state,
        options.registered_client_id,
    )?;
    Ok(AuthorizationGrant {
        callback,
        attempt,
        redirect_uri,
    })
}

fn wait_for_callback(
    listener: &TcpListener,
    port: u16,
    expected_state: &openidconnect::CsrfToken,
    registered_client_id: Option<&str>,
) -> Result<AuthorizationCallback, AuthError> {
    let deadline = Instant::now() + CALLBACK_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(AuthError::CallbackTimeout);
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                if !peer.ip().is_loopback() {
                    write_callback_response(&mut stream, 404, "Not found");
                    continue;
                }
                let Some(request) = read_callback_request(&mut stream, port) else {
                    write_callback_response(&mut stream, 400, "Invalid callback request");
                    continue;
                };
                let target = request.target;
                let Ok(url) =
                    openidconnect::url::Url::parse(&format!("http://127.0.0.1:{port}{target}"))
                else {
                    write_callback_response(&mut stream, 400, "Invalid callback request");
                    continue;
                };
                if url.path() != CALLBACK_PATH || url.query().is_none() {
                    write_callback_response(&mut stream, 404, "Not found");
                    continue;
                }
                let result = parse_callback_query(
                    url.query().unwrap_or_default(),
                    expected_state,
                    registered_client_id,
                );
                match result {
                    Ok(callback) => {
                        write_callback_response(
                            &mut stream,
                            200,
                            "Sign-in complete. Return to Carapanã.",
                        );
                        return Ok(callback);
                    }
                    Err(AuthError::StateMismatch | AuthError::InvalidCallback) => {
                        write_callback_response(
                            &mut stream,
                            400,
                            "Invalid sign-in state. Return to the original browser tab.",
                        );
                    }
                    Err(error) => {
                        write_callback_response(
                            &mut stream,
                            200,
                            "Sign-in did not complete. Return to Carapanã.",
                        );
                        return Err(error);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return Err(AuthError::CallbackUnavailable),
        }
    }
}

struct CallbackRequest {
    target: String,
}

fn read_callback_request(stream: &mut TcpStream, expected_port: u16) -> Option<CallbackRequest> {
    let mut reader = BufReader::new(stream);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;
    if first_line.len() > 4096 {
        return None;
    }
    let mut parts = first_line.split_ascii_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;
    if method != "GET"
        || version != "HTTP/1.1"
        || parts.next().is_some()
        || !target.starts_with('/')
    {
        return None;
    }
    let mut host = None;
    let mut total_bytes = first_line.len();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        total_bytes += header.len();
        if total_bytes > 8192 {
            return None;
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("host")
            && host.replace(value.trim().to_owned()).is_some()
        {
            return None;
        }
    }
    if host.as_deref() != Some(format!("127.0.0.1:{expected_port}").as_str()) {
        return None;
    }
    Some(CallbackRequest {
        target: target.to_owned(),
    })
}

fn write_callback_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 {
        "OK"
    } else if status == 404 {
        "Not Found"
    } else {
        "Bad Request"
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn open_system_browser(url: &str) -> Result<(), AuthError> {
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(AuthError::BrowserUnavailable);

    let mut child = Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AuthError::BrowserUnavailable)?;
    let _reaper = thread::Builder::new()
        .name("carapana-browser-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        })
        .map_err(|_| AuthError::BrowserUnavailable)?;
    Ok(())
}

fn exchange_authorization_code(
    metadata: &CoreProviderMetadata,
    http_client: &openidconnect::reqwest::blocking::Client,
    grant: AuthorizationGrant,
    expected_subject: Option<&str>,
) -> Result<ChatGptAccount, AuthError> {
    let client = CoreClient::from_provider_metadata(
        metadata.clone(),
        ClientId::new(grant.callback.client_id.clone()),
        None,
    )
    .set_redirect_uri(grant.redirect_uri);
    let token_response = client
        .exchange_code(AuthorizationCode::new(grant.callback.code))
        .map_err(|_| AuthError::TokenExchangeFailed)?
        .set_pkce_verifier(grant.attempt.verifier)
        .add_extra_param("resource", RESOURCE)
        .request(http_client)
        .map_err(|_| AuthError::TokenExchangeFailed)?;

    if !token_response
        .token_type()
        .as_ref()
        .eq_ignore_ascii_case("Bearer")
    {
        return Err(AuthError::InvalidTokenResponse);
    }
    let id_token = token_response
        .id_token()
        .ok_or(AuthError::InvalidTokenResponse)?;
    let verifier = client.id_token_verifier();
    let claims = id_token
        .claims(&verifier, &grant.attempt.nonce)
        .map_err(|_| AuthError::InvalidIdentity)?;
    if let Some(expected_hash) = claims.access_token_hash() {
        let signing_alg = id_token
            .signing_alg()
            .map_err(|_| AuthError::InvalidIdentity)?;
        let signing_key = id_token
            .signing_key(&verifier)
            .map_err(|_| AuthError::InvalidIdentity)?;
        let actual_hash =
            AccessTokenHash::from_token(token_response.access_token(), signing_alg, signing_key)
                .map_err(|_| AuthError::InvalidIdentity)?;
        if &actual_hash != expected_hash {
            return Err(AuthError::InvalidIdentity);
        }
    }

    let subject = claims.subject().as_str().to_owned();
    if expected_subject.is_some_and(|expected| expected != subject) {
        return Err(AuthError::AccountMismatch);
    }
    let expires_in = token_response
        .expires_in()
        .filter(|duration| !duration.is_zero())
        .ok_or(AuthError::InvalidTokenResponse)?;
    let expires_at = SystemTime::now()
        .checked_add(expires_in)
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .filter(|seconds| *seconds > 0)
        .ok_or(AuthError::InvalidTokenResponse)?;
    let scopes = token_response
        .scopes()
        .into_iter()
        .flatten()
        .map(|scope| scope.as_ref().to_owned())
        .collect::<Vec<_>>();
    let access_token = token_response.access_token().secret().to_owned();
    let refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_owned());
    let email = claims.email().map(|value| value.as_str().to_owned());
    let account = ChatGptAccount::new(ChatGptAccountData {
        client_id: grant.callback.client_id,
        subject,
        email,
        scopes,
        access_token,
        refresh_token,
        id_token: id_token.to_string(),
        expires_at_unix_seconds: expires_at,
    });
    Ok(account)
}

fn refresh_account(
    metadata: &CoreProviderMetadata,
    http_client: &openidconnect::reqwest::blocking::Client,
    account: &ChatGptAccount,
    refresh_token: &str,
) -> Result<ChatGptAccount, AuthError> {
    let client = CoreClient::from_provider_metadata(
        metadata.clone(),
        ClientId::new(account.client_id().to_owned()),
        None,
    );
    let token_response = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_owned()))
        .map_err(|_| AuthError::RefreshFailed)?
        .add_extra_param("resource", RESOURCE)
        .request(http_client)
        .map_err(|error| match error {
            openidconnect::RequestTokenError::ServerResponse(response)
                if is_terminal_refresh_error(response.error().as_ref()) =>
            {
                AuthError::RefreshRejected
            }
            _ => AuthError::RefreshFailed,
        })?;
    if !token_response
        .token_type()
        .as_ref()
        .eq_ignore_ascii_case("Bearer")
    {
        return Err(AuthError::InvalidTokenResponse);
    }
    let replacement_refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_owned())
        .filter(|token| !token.is_empty())
        .ok_or(AuthError::InvalidTokenResponse)?;
    let expires_in = token_response
        .expires_in()
        .filter(|duration| !duration.is_zero())
        .ok_or(AuthError::InvalidTokenResponse)?;
    let expires_at = SystemTime::now()
        .checked_add(expires_in)
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .filter(|seconds| *seconds > 0)
        .ok_or(AuthError::InvalidTokenResponse)?;

    let mut id_token = account.id_token().to_owned();
    let mut email = account.email().map(str::to_owned);
    if let Some(refreshed_id_token) = token_response.id_token() {
        let verifier = client.id_token_verifier();
        let claims = refreshed_id_token
            .claims(&verifier, |_: Option<&openidconnect::Nonce>| Ok(()))
            .map_err(|_| AuthError::InvalidIdentity)?;
        if claims.subject().as_str() != account.subject() {
            return Err(AuthError::AccountMismatch);
        }
        if let Some(expected_hash) = claims.access_token_hash() {
            let signing_alg = refreshed_id_token
                .signing_alg()
                .map_err(|_| AuthError::InvalidIdentity)?;
            let signing_key = refreshed_id_token
                .signing_key(&verifier)
                .map_err(|_| AuthError::InvalidIdentity)?;
            let actual_hash = AccessTokenHash::from_token(
                token_response.access_token(),
                signing_alg,
                signing_key,
            )
            .map_err(|_| AuthError::InvalidIdentity)?;
            if &actual_hash != expected_hash {
                return Err(AuthError::InvalidIdentity);
            }
        }
        id_token = refreshed_id_token.to_string();
        if let Some(refreshed_email) = claims.email() {
            email = Some(refreshed_email.as_str().to_owned());
        }
    }

    let scopes = match token_response.scopes() {
        Some(scopes) => scopes
            .iter()
            .map(|scope| scope.as_ref().to_owned())
            .collect(),
        None => account.scopes().to_vec(),
    };
    Ok(ChatGptAccount::new(ChatGptAccountData {
        client_id: account.client_id().to_owned(),
        subject: account.subject().to_owned(),
        email,
        scopes,
        access_token: token_response.access_token().secret().to_owned(),
        refresh_token: Some(replacement_refresh_token),
        id_token,
        expires_at_unix_seconds: expires_at,
    }))
}

fn is_terminal_refresh_error(code: &str) -> bool {
    matches!(
        code,
        "invalid_grant"
            | "invalid_refresh_token"
            | "token_expired"
            | "refresh_token_expired"
            | "refresh_token_invalidated"
            | "refresh_token_reused"
    )
}

fn parse_callback_query(
    query: &str,
    expected_state: &openidconnect::CsrfToken,
    registered_client_id: Option<&str>,
) -> Result<AuthorizationCallback, AuthError> {
    let mut returned_state = None;
    let mut code = None;
    let mut returned_client_id = None;
    let mut authorization_error = None;

    for (key, value) in openidconnect::url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "state" => {
                if returned_state.replace(value.into_owned()).is_some() {
                    return Err(AuthError::InvalidCallback);
                }
            }
            "code" => {
                if code.replace(value.into_owned()).is_some() {
                    return Err(AuthError::InvalidCallback);
                }
            }
            "client_id" => {
                if returned_client_id.replace(value.into_owned()).is_some() {
                    return Err(AuthError::InvalidCallback);
                }
            }
            "error" if authorization_error.replace(value.into_owned()).is_some() => {
                return Err(AuthError::InvalidCallback);
            }
            "error" => {}
            _ => {}
        }
    }

    let returned_state = returned_state.ok_or(AuthError::StateMismatch)?;
    if openidconnect::CsrfToken::new(returned_state) != *expected_state {
        return Err(AuthError::StateMismatch);
    }

    if let Some(error) = authorization_error {
        return if error == "access_denied" {
            Err(AuthError::AuthorizationDenied)
        } else {
            Err(AuthError::AuthorizationFailed)
        };
    }

    let client_id = match (registered_client_id, returned_client_id) {
        (Some(existing), Some(returned)) if existing != returned => {
            return Err(AuthError::ClientIdMismatch);
        }
        (Some(existing), _) => existing.to_owned(),
        (None, Some(returned)) => returned,
        (None, None) => return Err(AuthError::InvalidClientId),
    };
    if !valid_client_id(&client_id) {
        return Err(AuthError::InvalidClientId);
    }

    let code = code
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .ok_or(AuthError::InvalidCallback)?;

    Ok(AuthorizationCallback { code, client_id })
}

fn valid_client_id(client_id: &str) -> bool {
    !client_id.is_empty()
        && client_id.len() <= 200
        && client_id != "dynamic_agent_client"
        && client_id.bytes().all(|byte| byte.is_ascii_graphic())
}

fn valid_host_id(host_id: &str) -> bool {
    host_id
        .strip_prefix("urn:uuid:")
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .is_some_and(|value| value.get_version_num() == 4)
}

fn authorization_attempt(
    metadata: &CoreProviderMetadata,
    redirect_uri: RedirectUrl,
    options: &AuthorizationOptions<'_>,
) -> AuthorizationAttempt {
    let client_id = options.registered_client_id.unwrap_or(DYNAMIC_CLIENT_ID);
    let client = CoreClient::from_provider_metadata(
        metadata.clone(),
        openidconnect::ClientId::new(client_id.to_owned()),
        None,
    )
    .set_redirect_uri(redirect_uri);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            openidconnect::CsrfToken::new_random,
            openidconnect::Nonce::new_random,
        )
        .set_pkce_challenge(challenge)
        .add_extra_param("resource", RESOURCE)
        .add_extra_param("ext_agent_host_id", options.host_id);
    for scope in REQUESTED_SCOPES {
        request = request.add_scope(Scope::new(scope.to_owned()));
    }
    if options.registered_client_id.is_none() {
        request = request.add_extra_param("agent_name_hint", options.app_name);
    }
    if let Some(email) = options.login_hint {
        request = request.add_extra_param("login_hint", email);
    }
    if let Some(id_token) = options.id_token_hint {
        request = request.add_extra_param("id_token_hint", id_token);
    }
    if options.request_consent {
        request = request.add_extra_param("prompt", "consent");
    }
    let (authorization_url, state, nonce) = request.url();

    AuthorizationAttempt {
        authorization_url,
        state,
        nonce,
        verifier,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthError, AuthorizationOptions, RESOURCE, authorization_attempt, authorization_grant,
        is_terminal_refresh_error, parse_callback_query, refresh_account,
    };
    use crate::{ChatGptAccount, account::ChatGptAccountData};
    use openidconnect::{
        AuthUrl, EmptyAdditionalProviderMetadata, IssuerUrl, JsonWebKeySetUrl, OAuth2TokenResponse,
        RedirectUrl, ResponseTypes, TokenUrl,
        core::{
            CoreJwsSigningAlgorithm, CoreProviderMetadata, CoreResponseType,
            CoreSubjectIdentifierType,
        },
    };
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    fn fake_provider_metadata() -> CoreProviderMetadata {
        CoreProviderMetadata::new(
            IssuerUrl::new("https://auth.openai.com".to_owned()).unwrap(),
            AuthUrl::new("https://auth.openai.com/api/accounts/authorize".to_owned()).unwrap(),
            JsonWebKeySetUrl::new("https://auth.openai.com/.well-known/jwks.json".to_owned())
                .unwrap(),
            vec![ResponseTypes::new(vec![CoreResponseType::Code])],
            vec![CoreSubjectIdentifierType::Public],
            vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            EmptyAdditionalProviderMetadata {},
        )
        .set_token_endpoint(Some(
            TokenUrl::new("https://auth.openai.com/api/accounts/oauth/token".to_owned()).unwrap(),
        ))
    }

    #[test]
    fn initial_authorization_requests_official_scopes_and_pkce() {
        let redirect_uri =
            RedirectUrl::new("http://127.0.0.1:1455/auth/callback".to_owned()).unwrap();
        let attempt = authorization_attempt(
            &fake_provider_metadata(),
            redirect_uri,
            &AuthorizationOptions {
                registered_client_id: None,
                host_id: "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
                id_token_hint: None,
                login_hint: None,
                app_name: "Carapanã",
                request_consent: false,
            },
        );
        let parameters = attempt
            .authorization_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            parameters.get("client_id"),
            Some(&"dynamic_agent_client".to_owned())
        );
        assert_eq!(parameters.get("resource"), Some(&RESOURCE.to_owned()));
        assert_eq!(
            parameters.get("ext_agent_host_id"),
            Some(&"urn:uuid:550e8400-e29b-41d4-a716-446655440000".to_owned())
        );
        assert_eq!(
            parameters.get("agent_name_hint"),
            Some(&"Carapanã".to_owned())
        );
        assert_eq!(
            parameters.get("redirect_uri"),
            Some(&"http://127.0.0.1:1455/auth/callback".to_owned())
        );
        assert_eq!(parameters.get("response_type"), Some(&"code".to_owned()));
        assert_eq!(
            parameters.get("code_challenge_method"),
            Some(&"S256".to_owned())
        );
        assert!(
            parameters
                .get("code_challenge")
                .is_some_and(|value| !value.is_empty())
        );
        let scopes = parameters
            .get("scope")
            .unwrap()
            .split(' ')
            .collect::<Vec<_>>();
        for expected in [
            "openid",
            "profile",
            "email",
            "offline_access",
            "resource.invoke",
            "chatgpt.tokens.use.direct",
        ] {
            assert!(scopes.contains(&expected));
        }
        assert!(!attempt.state.secret().is_empty());
        assert!(!attempt.nonce.secret().is_empty());
        assert!(attempt.verifier.secret().len() >= 43);
    }

    #[test]
    fn returning_authorization_reuses_client_and_omits_registration_hint() {
        let attempt = authorization_attempt(
            &fake_provider_metadata(),
            RedirectUrl::new("http://127.0.0.1:54321/auth/callback".to_owned()).unwrap(),
            &AuthorizationOptions {
                registered_client_id: Some("oaiapp_existing"),
                host_id: "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
                id_token_hint: Some("synthetic-retained-id-token"),
                login_hint: Some("account@example.invalid"),
                app_name: "Carapanã",
                request_consent: false,
            },
        );
        let parameters = attempt
            .authorization_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            parameters.get("client_id"),
            Some(&"oaiapp_existing".to_owned())
        );
        assert!(!parameters.contains_key("agent_name_hint"));
        assert_eq!(
            parameters.get("id_token_hint"),
            Some(&"synthetic-retained-id-token".to_owned())
        );
        assert_eq!(
            parameters.get("login_hint"),
            Some(&"account@example.invalid".to_owned())
        );
        assert!(!parameters.contains_key("prompt"));
        assert_eq!(
            parameters.get("redirect_uri"),
            Some(&"http://127.0.0.1:54321/auth/callback".to_owned())
        );
    }

    #[test]
    fn explicit_plan_enablement_requests_consent_without_changing_client_registration() {
        let attempt = authorization_attempt(
            &fake_provider_metadata(),
            RedirectUrl::new("http://127.0.0.1:54321/auth/callback".to_owned()).unwrap(),
            &AuthorizationOptions {
                registered_client_id: Some("oaiapp_existing"),
                host_id: "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
                id_token_hint: None,
                login_hint: None,
                app_name: "Carapanã",
                request_consent: true,
            },
        );
        let parameters = attempt
            .authorization_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            parameters.get("client_id"),
            Some(&"oaiapp_existing".to_owned())
        );
        assert_eq!(parameters.get("prompt"), Some(&"consent".to_owned()));
        assert!(!parameters.contains_key("agent_name_hint"));
        assert!(parameters.get("scope").is_some_and(|scope| {
            scope
                .split(' ')
                .any(|value| value == "chatgpt.tokens.use.direct")
        }));
    }

    #[test]
    fn only_terminal_refresh_codes_invalidate_saved_credentials() {
        for terminal_code in [
            "invalid_grant",
            "invalid_refresh_token",
            "token_expired",
            "refresh_token_expired",
            "refresh_token_invalidated",
            "refresh_token_reused",
        ] {
            assert!(is_terminal_refresh_error(terminal_code));
        }
        assert!(!is_terminal_refresh_error("server_error"));
        assert!(!is_terminal_refresh_error("temporarily_unavailable"));
    }

    #[test]
    fn callback_requires_state_and_reuses_issued_client_id() {
        let expected_state = openidconnect::CsrfToken::new("fresh-state".to_owned());
        let callback = parse_callback_query(
            "code=one-time-code&state=fresh-state",
            &expected_state,
            Some("oaiapp_existing"),
        )
        .unwrap();

        assert_eq!(callback.client_id, "oaiapp_existing");
        assert!(matches!(
            parse_callback_query(
                "code=one-time-code&state=wrong-state&client_id=oaiapp_other",
                &expected_state,
                None,
            ),
            Err(AuthError::StateMismatch)
        ));
    }

    #[test]
    fn registration_callback_requires_a_valid_issued_client_id() {
        let expected_state = openidconnect::CsrfToken::new("fresh-state".to_owned());
        assert!(matches!(
            parse_callback_query(
                "code=one-time-code&state=fresh-state&client_id=dynamic_agent_client",
                &expected_state,
                None,
            ),
            Err(AuthError::InvalidClientId)
        ));
        assert!(matches!(
            parse_callback_query(
                "code=one-time-code&state=fresh-state&client_id=oaiapp_other",
                &expected_state,
                Some("oaiapp_existing"),
            ),
            Err(AuthError::ClientIdMismatch)
        ));
    }

    #[test]
    fn loopback_callback_rejects_wrong_state_then_accepts_the_active_registration() {
        let sender_thread = Arc::new(Mutex::new(None::<JoinHandle<()>>));
        let sender_thread_for_browser = Arc::clone(&sender_thread);
        let grant = authorization_grant(
            &fake_provider_metadata(),
            &AuthorizationOptions {
                registered_client_id: None,
                host_id: "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
                id_token_hint: None,
                login_hint: None,
                app_name: "Carapanã",
                request_consent: false,
            },
            move |authorization_url| {
                let authorization_url = openidconnect::url::Url::parse(authorization_url).unwrap();
                let redirect_uri = authorization_url
                    .query_pairs()
                    .find(|(key, _)| key == "redirect_uri")
                    .map(|(_, value)| openidconnect::url::Url::parse(&value).unwrap())
                    .unwrap();
                let port = redirect_uri.port().unwrap();
                let query = authorization_url
                    .query_pairs()
                    .find(|(key, _)| key == "state")
                    .map(|(_, value)| value.into_owned())
                    .unwrap();
                let sender = std::thread::spawn(move || {
                    let wrong = send_callback(
                        port,
                        "state=not-the-current-state&code=wrong&client_id=oaiapp_test",
                    );
                    assert!(wrong.starts_with("HTTP/1.1 400"));
                    let valid = send_callback(
                        port,
                        &format!("state={query}&code=one-time-code&client_id=oaiapp_issued"),
                    );
                    assert!(valid.starts_with("HTTP/1.1 200"));
                });
                *sender_thread_for_browser.lock().unwrap() = Some(sender);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(grant.callback.client_id, "oaiapp_issued");
        assert_eq!(grant.callback.code, "one-time-code");
        sender_thread
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn refresh_rotates_token_with_issued_client_and_resource_without_reducing_scope() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = r#"{"access_token":"replacement-access","refresh_token":"replacement-refresh","token_type":"Bearer","expires_in":3600,"scope":"openid profile chatgpt.tokens.use.direct"}"#;
        let parsed_response =
            serde_json::from_str::<openidconnect::core::CoreTokenResponse>(response_body).unwrap();
        assert!(
            parsed_response
                .token_type()
                .as_ref()
                .eq_ignore_ascii_case("Bearer")
        );
        assert_eq!(
            parsed_response
                .refresh_token()
                .map(|token| token.secret().as_str()),
            Some("replacement-refresh")
        );
        assert_eq!(
            parsed_response.expires_in(),
            Some(Duration::from_secs(3600))
        );
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            )
            .unwrap();
            request
        });
        let metadata = fake_provider_metadata_with_token_url(&format!("http://{address}/token"));
        let http_client = super::oidc_http_client().unwrap();
        let account = ChatGptAccount::new(ChatGptAccountData {
            client_id: "oaiapp_refresh-test".to_owned(),
            subject: "subject-refresh-test".to_owned(),
            email: Some("account@example.invalid".to_owned()),
            scopes: vec![
                "openid".to_owned(),
                "profile".to_owned(),
                "chatgpt.tokens.use.direct".to_owned(),
            ],
            access_token: "old-access".to_owned(),
            refresh_token: Some("old-refresh".to_owned()),
            id_token: "retained-id-token".to_owned(),
            expires_at_unix_seconds: 4_000_000_000,
        });

        let refreshed = refresh_account(&metadata, &http_client, &account, "old-refresh").unwrap();
        let wire_request = server.join().unwrap();
        let (_, request_body) = split_http_request(&wire_request);
        let parameters = openidconnect::url::form_urlencoded::parse(request_body)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(refreshed.access_token(), "replacement-access");
        assert_eq!(refreshed.refresh_token(), Some("replacement-refresh"));
        assert!(refreshed.plan_usage_enabled());
        assert_eq!(
            parameters.get("grant_type"),
            Some(&"refresh_token".to_owned())
        );
        assert_eq!(
            parameters.get("client_id"),
            Some(&"oaiapp_refresh-test".to_owned())
        );
        assert_eq!(
            parameters.get("refresh_token"),
            Some(&"old-refresh".to_owned())
        );
        assert_eq!(parameters.get("resource"), Some(&RESOURCE.to_owned()));
        assert!(!parameters.contains_key("scope"));
    }

    #[test]
    fn invalid_grant_is_reported_as_a_terminal_refresh_rejection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let body = r#"{"error":"invalid_grant"}"#;
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        });
        let metadata = fake_provider_metadata_with_token_url(&format!("http://{address}/token"));
        let http_client = super::oidc_http_client().unwrap();
        let account = ChatGptAccount::new(ChatGptAccountData {
            client_id: "oaiapp_refresh-test".to_owned(),
            subject: "subject-refresh-test".to_owned(),
            email: None,
            scopes: vec!["chatgpt.tokens.use.direct".to_owned()],
            access_token: "old-access".to_owned(),
            refresh_token: Some("old-refresh".to_owned()),
            id_token: "retained-id-token".to_owned(),
            expires_at_unix_seconds: 4_000_000_000,
        });

        assert!(matches!(
            refresh_account(&metadata, &http_client, &account, "old-refresh"),
            Err(AuthError::RefreshRejected)
        ));
        let _ = server.join().unwrap();
    }

    fn fake_provider_metadata_with_token_url(token_url: &str) -> CoreProviderMetadata {
        CoreProviderMetadata::new(
            IssuerUrl::new("https://auth.openai.com".to_owned()).unwrap(),
            AuthUrl::new("https://auth.openai.com/api/accounts/authorize".to_owned()).unwrap(),
            JsonWebKeySetUrl::new("https://auth.openai.com/.well-known/jwks.json".to_owned())
                .unwrap(),
            vec![ResponseTypes::new(vec![CoreResponseType::Code])],
            vec![CoreSubjectIdentifierType::Public],
            vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            EmptyAdditionalProviderMetadata {},
        )
        .set_token_endpoint(Some(TokenUrl::new(token_url.to_owned()).unwrap()))
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&chunk[..count]);
            let Some((header_end, content_length)) = request_content_length(&request) else {
                continue;
            };
            if request.len() >= header_end + content_length {
                return String::from_utf8(request).unwrap();
            }
        }
    }

    fn request_content_length(request: &[u8]) -> Option<(usize, usize)> {
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")?
            + 4;
        let headers = std::str::from_utf8(&request[..header_end]).ok()?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        Some((header_end, content_length))
    }

    fn split_http_request(request: &str) -> (&str, &[u8]) {
        let header_end = request.find("\r\n\r\n").unwrap() + 4;
        (&request[..header_end], &request.as_bytes()[header_end..])
    }

    fn send_callback(port: u16, query: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET /auth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
}
