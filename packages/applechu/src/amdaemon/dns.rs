impl DnsConfig {
    pub fn requires_localhost_patch(&self) -> bool {
        [
            &self.default,
            &self.router,
            &self.startup,
            &self.billing,
            &self.aimedb,
            &self.title,
        ]
        .into_iter()
        .any(|target| is_loopback_target(target))
    }
}

fn is_loopback_target(target: &str) -> bool {
    let target = target.trim();
    let authority = target
        .split_once("://")
        .map_or(target, |(_, authority)| authority)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = endpoint_host(authority).trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || host
            .to_ascii_lowercase()
            .strip_suffix(".localhost")
            .is_some_and(|prefix| !prefix.is_empty())
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn endpoint_host(authority: &str) -> &str {
    if authority.parse::<std::net::IpAddr>().is_ok() {
        return authority;
    }
    if let Some(bracketed) = authority.strip_prefix('[') {
        return bracketed
            .split_once(']')
            .map_or(bracketed, |(host, _)| host);
    }
    authority
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(authority, |(host, _)| host)
}

crate::config_section! {
    pub struct DnsConfig => DNS_CONFIG_SECTION {
        section: "Dns",
        order: 30,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "服务器地址",
        fields: {
            pub default: String = String::from("play.mumur.net"),
            emit_default: true;
            pub aimedb: String = String::from("aime.mumur.net"),
            emit_default: true;
            pub router: String = String::new(),
            advanced: true;
            pub startup: String = String::new(),
            advanced: true;
            pub billing: String = String::new(),
            advanced: true;
            pub title: String = String::new(),
            advanced: true;
            pub replace_host: bool = true,
            advanced: true,
            comment: "替换 HTTP Host";
            pub startup_port: u16 = 0,
            advanced: true;
            pub billing_port: u16 = 0,
            advanced: true;
            pub aimedb_port: u16 = 0,
            advanced: true;
        }
    }
}
