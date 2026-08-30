use rama::{
    Service,
    error::extra::OpaqueError,
    http::{
        HeaderValue, Request, Response, StatusCode,
        client::EasyHttpWebClient,
        header::{AUTHORIZATION, LOCATION, USER_AGENT},
        service::client::HttpClientExt,
    },
    rt::Executor,
    service::BoxService,
    tls::client::TlsClientConfig,
};

pub type RamaClient = BoxService<Request, Response, OpaqueError>;

const MAX_REDIRECTS: usize = 10;

#[derive(Debug)]
pub enum RedirectError {
    Request(String),
    MissingLocation(StatusCode),
    InvalidLocationHeader,
    TooManyRedirects {
        url: String,
    },
    InvalidBaseUrl {
        url: String,
        source: url::ParseError,
    },
    InvalidLocation {
        location: String,
        source: url::ParseError,
    },
}

#[cfg(target_os = "macos")]
pub fn redirect_error_message(error: RedirectError) -> String {
    redirect_error_message_with_request_context(error, None)
}

pub fn redirect_error_message_with_request_context(
    error: RedirectError,
    request_context: Option<&str>,
) -> String {
    match error {
        RedirectError::Request(message) => match request_context {
            Some(context) => format!("{context}: {message}"),
            None => message,
        },
        RedirectError::MissingLocation(status) => {
            format!("redirect ({status}) without Location header")
        }
        RedirectError::InvalidLocationHeader => {
            "redirect Location header contains invalid characters".to_string()
        }
        RedirectError::TooManyRedirects { url } => {
            format!("too many redirects while fetching {url}")
        }
        RedirectError::InvalidBaseUrl { url, source } => {
            format!("invalid redirect base URL '{url}': {source}")
        }
        RedirectError::InvalidLocation { location, source } => {
            format!("invalid redirect location '{location}': {source}")
        }
    }
}

#[derive(Default)]
pub struct RedirectHeaders {
    pub authorization: Option<HeaderValue>,
    pub range: Option<String>,
    pub user_agent: Option<HeaderValue>,
}

pub fn build_rama_client() -> RamaClient {
    EasyHttpWebClient::connector_builder()
        .with_default_transport_connector()
        .with_default_dns_connector()
        .without_tls_proxy_support()
        .with_proxy_support()
        .with_tls_support_using_rustls(TlsClientConfig::default_http())
        .with_default_http_connector(Executor::default())
        .with_default_connection_pool()
        .build_client()
        .boxed()
}

#[cfg(target_os = "macos")]
pub fn build_isolated_rama_client() -> RamaClient {
    EasyHttpWebClient::connector_builder()
        .with_default_transport_connector()
        .with_default_dns_connector()
        .without_tls_proxy_support()
        .with_proxy_support()
        .with_tls_support_using_rustls(TlsClientConfig::default_http())
        .with_default_http_connector(Executor::default())
        .without_connection_pool()
        .build_client()
        .boxed()
}

pub async fn send_get_with_redirects(
    client: &RamaClient,
    url: &str,
    headers: RedirectHeaders,
) -> Result<Response, RedirectError> {
    send_with_redirects(client, url, headers, RequestMethod::Get).await
}

#[cfg(target_os = "macos")]
pub async fn send_head_with_redirects(
    client: &RamaClient,
    url: &str,
    headers: RedirectHeaders,
) -> Result<Response, RedirectError> {
    send_with_redirects(client, url, headers, RequestMethod::Head).await
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum RequestMethod {
    Get,
    Head,
}

async fn send_with_redirects(
    client: &RamaClient,
    url: &str,
    headers: RedirectHeaders,
    method: RequestMethod,
) -> Result<Response, RedirectError> {
    let mut current_url = url.to_string();
    let mut redirects = 0usize;

    loop {
        let mut request = match method {
            RequestMethod::Get => client.get(&current_url),
            RequestMethod::Head => client.head(&current_url),
        };

        if let Some(auth) = headers.authorization.clone() {
            request = request.header(AUTHORIZATION, auth);
        }
        if let Some(ref range) = headers.range {
            request = request.header("Range", range.as_str());
        }
        if let Some(user_agent) = headers.user_agent.clone() {
            request = request.header(USER_AGENT, user_agent);
        }

        let response = request
            .send()
            .await
            .map_err(|e| RedirectError::Request(e.to_string()))?;

        if !response.status().is_redirection() {
            return Ok(response);
        }

        redirects += 1;
        if redirects > MAX_REDIRECTS {
            return Err(RedirectError::TooManyRedirects {
                url: url.to_string(),
            });
        }

        let location = redirect_location(&response)?;
        current_url = resolve_redirect_url(&current_url, &location)?;
    }
}

fn redirect_location(response: &Response) -> Result<String, RedirectError> {
    let Some(location) = response.headers().get(LOCATION) else {
        return Err(RedirectError::MissingLocation(response.status()));
    };

    location
        .to_str()
        .map(ToString::to_string)
        .map_err(|_| RedirectError::InvalidLocationHeader)
}

fn resolve_redirect_url(current_url: &str, location: &str) -> Result<String, RedirectError> {
    let base = url::Url::parse(current_url).map_err(|e| RedirectError::InvalidBaseUrl {
        url: current_url.to_string(),
        source: e,
    })?;
    let next = base
        .join(location)
        .map_err(|e| RedirectError::InvalidLocation {
            location: location.to_string(),
            source: e,
        })?;
    Ok(next.to_string())
}
