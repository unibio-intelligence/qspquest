//! Deployment boundaries for local stdio and public or managed HTTP.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Local,
    Public,
    Managed,
}
impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Public => "public",
            Self::Managed => "managed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Network {
    address: IpAddr,
    prefix: u8,
}
impl Network {
    pub fn parse(value: &str) -> Result<Self, String> {
        let (address, prefix) = value.split_once('/').unwrap_or((value, ""));
        let address: IpAddr = address
            .parse()
            .map_err(|_| "Trusted proxies require IP addresses or CIDR networks")?;
        let width = if address.is_ipv4() { 32 } else { 128 };
        let prefix = if prefix.is_empty() {
            width
        } else {
            prefix.parse().map_err(|_| "Invalid trusted-proxy prefix")?
        };
        if prefix == 0 || prefix > width {
            return Err(
                "Trusted-proxy prefixes must restrict ingress; 0/0 is not supported".into(),
            );
        }
        Ok(Self { address, prefix })
    }
    pub fn contains(&self, peer: IpAddr) -> bool {
        match (self.address, peer) {
            (IpAddr::V4(network), IpAddr::V4(peer)) => {
                let mask = u32::MAX << (32 - self.prefix);
                u32::from(network) & mask == u32::from(peer) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(peer)) => {
                let mask = u128::MAX << (128 - self.prefix);
                u128::from(network) & mask == u128::from(peer) & mask
            }
            _ => false,
        }
    }
}

/// Validate outbound endpoints without reflecting credentials into diagnostics.
pub fn endpoint(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Endpoint must be an absolute HTTPS URL")?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Endpoint requires HTTPS (loopback HTTP for local tests), without userinfo, query or fragment".into());
    }
    Ok(url)
}

fn list(value: Option<&str>) -> Result<Vec<&str>, String> {
    value.map_or_else(
        || Ok(vec![]),
        |value| {
            let items: Vec<_> = value.split(',').map(str::trim).collect();
            if items.iter().any(|v| v.is_empty()) {
                Err("Allowlist entries cannot be empty".into())
            } else {
                Ok(items)
            }
        },
    )
}

fn origin(value: &str) -> Result<String, String> {
    let url = endpoint(value)?;
    if url.path() != "/" {
        return Err("Browser and gateway origins must not contain a path".into());
    }
    // Explicit default ports avoid SDK rules that allow any port when omitted.
    Ok(format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap(),
        url.port_or_known_default().unwrap()
    ))
}

#[derive(Default)]
pub struct Inputs<'a> {
    pub requested_profile: Option<&'a str>,
    pub managed_enabled: Option<&'a str>,
    pub allowed_hosts: Option<&'a str>,
    pub allowed_origins: Option<&'a str>,
    pub gateway_origin: Option<&'a str>,
    pub trusted_proxies: Option<&'a str>,
}

pub struct Settings {
    pub profile: Profile,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub trusted_proxies: Vec<Network>,
    pub external_tls: bool,
    pub loopback_listener: bool,
}
impl Settings {
    pub fn parse(http: bool, loopback_listener: bool, input: Inputs<'_>) -> Result<Self, String> {
        let managed = match input.managed_enabled {
            None | Some("0") => false,
            Some("1") => true,
            _ => return Err("QSPQUEST_MANAGED_ENABLED must be 0 or 1".into()),
        };
        let profile = match input.requested_profile {
            Some("local") => Profile::Local,
            Some("public") => Profile::Public,
            Some("managed") => Profile::Managed,
            None if managed => Profile::Managed,
            None if http => Profile::Public,
            None => Profile::Local,
            _ => return Err("Profile must be local, public or managed".into()),
        };
        if http == (profile == Profile::Local) {
            return Err(
                "Local profile uses stdio; public and managed profiles require --http ADDR".into(),
            );
        }
        if (managed && profile != Profile::Managed)
            || (profile == Profile::Managed && input.managed_enabled == Some("0"))
        {
            return Err("Profile conflicts with QSPQUEST_MANAGED_ENABLED; choose one consistent deployment mode".into());
        }
        let mut allowed_hosts = vec!["localhost".into(), "127.0.0.1".into(), "[::1]".into()];
        if let Some(value) = input.allowed_hosts {
            allowed_hosts = list(Some(value))?.into_iter().map(|host| {
                let host = if host == "::1" { "[::1]" } else { host };
                let url = reqwest::Url::parse(&format!("http://{host}"))
                    .map_err(|_| "Allowed hosts require a hostname or IP, with an optional port")?;
                if host.contains('*') || host.contains('/') || host.contains('\\') || host.chars().any(char::is_whitespace) || !url.username().is_empty()
                    || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || url.path() != "/"
                { return Err("Allowed hosts cannot contain wildcards, userinfo, paths, queries or fragments".to_owned()); }
                Ok(host.to_owned())
            }).collect::<Result<_, String>>()?;
        }
        let allowed_origins = list(input.allowed_origins)?
            .into_iter()
            .map(origin)
            .collect::<Result<Vec<_>, _>>()?;
        let external_tls = if let Some(value) = input.gateway_origin {
            let value = origin(value)?;
            if !value.starts_with("https://") {
                return Err("Gateway origin must use HTTPS".into());
            }
            true
        } else {
            false
        };
        let mut trusted_proxies = list(input.trusted_proxies)?
            .into_iter()
            .map(Network::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if profile == Profile::Managed {
            if !loopback_listener && (!external_tls || trusted_proxies.is_empty()) {
                return Err("Non-loopback managed HTTP requires QSPQUEST_MCP_GATEWAY_ORIGIN=https://HOST and QSPQUEST_MCP_TRUSTED_PROXIES=IP_OR_CIDR; restrict the backend port to that gateway".into());
            }
            if trusted_proxies.is_empty() {
                trusted_proxies = vec![Network::parse("127.0.0.0/8")?, Network::parse("::1/128")?];
            }
        }
        Ok(Self {
            profile,
            allowed_hosts,
            allowed_origins,
            trusted_proxies,
            external_tls,
            loopback_listener,
        })
    }
    pub fn diagnostics(&self) {
        eprintln!("Deployment profile: {}; listener: {}; TLS: {}; browser Origin validation: {}; forwarded identity/proxy headers: ignored",
            self.profile.name(), if self.loopback_listener { "loopback" } else { "non-loopback" },
            if self.profile == Profile::Local { "not applicable" } else if self.external_tls { "external HTTPS gateway" } else { "plain HTTP" }, if self.profile == Profile::Local { "not applicable" } else { "enforced" });
        if self.profile != Profile::Local {
            eprintln!(
                "MCP Host allowlist: {}; browser Origins: {}; trusted proxy networks: {}",
                self.allowed_hosts.join(","),
                if self.allowed_origins.is_empty() {
                    "none (requests carrying Origin are refused)".into()
                } else {
                    self.allowed_origins.join(",")
                },
                self.trusted_proxies.len()
            );
        }
        if self.profile == Profile::Public {
            eprintln!("Public compute has no managed authentication; model inputs and results travel to this HTTP server.");
        }
        if self.profile == Profile::Managed {
            eprintln!("Managed MCP requires verified sessions and restricts ingress to configured proxy peers.");
        }
    }
}

/// Actual peer addresses enforce ingress; caller-supplied forwarded headers do not.
pub async fn proxy_ingress(
    axum::extract::State(networks): axum::extract::State<std::sync::Arc<Vec<Network>>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !request.uri().path().starts_with("/mcp") {
        return next.run(request).await;
    }
    let allowed = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .is_some_and(|peer| networks.iter().any(|network| network.contains(peer.0.ip())));
    if !allowed {
        return (
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(crate::policy::error(
                "ingress_denied",
                "MCP ingress requires a configured gateway peer",
            )),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_fail_closed_and_defaults_remain_local() {
        assert_eq!(
            Settings::parse(false, true, Inputs::default())
                .unwrap()
                .profile,
            Profile::Local
        );
        assert_eq!(
            Settings::parse(true, true, Inputs::default())
                .unwrap()
                .profile,
            Profile::Public
        );
        assert!(Settings::parse(
            false,
            true,
            Inputs {
                managed_enabled: Some("1"),
                ..Inputs::default()
            }
        )
        .is_err());
        assert!(Settings::parse(
            true,
            false,
            Inputs {
                requested_profile: Some("managed"),
                ..Inputs::default()
            }
        )
        .is_err());
        let valid = Settings::parse(
            true,
            false,
            Inputs {
                requested_profile: Some("managed"),
                gateway_origin: Some("https://mcp.example.test"),
                trusted_proxies: Some("10.2.3.0/24"),
                allowed_origins: Some("https://app.example.test"),
                ..Inputs::default()
            },
        )
        .unwrap();
        assert_eq!(valid.allowed_origins, vec!["https://app.example.test:443"]);
        assert!(valid.trusted_proxies[0].contains("10.2.3.4".parse().unwrap()));
        assert!(!valid.trusted_proxies[0].contains("10.2.4.4".parse().unwrap()));
        assert!(Network::parse("0.0.0.0/0").is_err());
        assert!(Network::parse("::/0").is_err());
        assert!(endpoint("https://credential@example.test/session").is_err());
        assert!(endpoint("http://public.example.test/session").is_err());
        assert!(Settings::parse(
            true,
            true,
            Inputs {
                allowed_hosts: Some("example.test/"),
                ..Inputs::default()
            }
        )
        .is_err());
        assert!(Settings::parse(
            true,
            true,
            Inputs {
                allowed_hosts: Some("*"),
                ..Inputs::default()
            }
        )
        .is_err());
    }
}
