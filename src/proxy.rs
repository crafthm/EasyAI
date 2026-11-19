//! Automatic system proxy detection.
//!
//! reqwest only honours `HTTP(S)_PROXY` environment variables, not the GNOME
//! desktop proxy settings. Resolve the desktop proxy once at startup and put it
//! in the environment for the HTTP client. The webview gets the opposite
//! treatment: it only loads local content, so it is pointed at a proxy-free
//! network stack (see [`disable_webview_proxy`]).

/// Applies the desktop proxy settings to this process's environment.
///
/// An explicit `HTTP(S)_PROXY`/`ALL_PROXY` environment variable always wins.
pub fn init() {
    #[cfg(target_os = "linux")]
    {
        // Keep localhost traffic (the dev server, local recipe URLs) direct.
        // WebKit resolves proxies through libproxy, which also proxies
        // localhost; without this exclusion the GTK webview cannot load the
        // Vite dev server and reports `internallyFailedLoadTimerFired`.
        ensure_no_proxy();
        if has_env_proxy() {
            return;
        }
        let Some(proxy) = desktop_proxy() else {
            return;
        };
        for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
            std::env::set_var(key, &proxy);
        }
    }
}

/// Makes the webview ignore the desktop proxy.
///
/// The webview only ever loads local content: the Vite dev server in
/// development, bundled assets in release. WebKitGTK 2.50 breaks every load,
/// including the Vite HMR WebSocket, when a GNOME system proxy is configured,
/// so the webview gets its own proxy-free network stack. The Rust HTTP client
/// keeps using the desktop proxy through the environment set by [`init`].
#[cfg(target_os = "linux")]
pub fn disable_webview_proxy(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| {
        use webkit2gtk::{WebContextExt, WebViewExt, WebsiteDataManagerExt};

        let Some(context) = webview.inner().context() else {
            return;
        };
        let Some(manager) = context.website_data_manager() else {
            return;
        };
        manager.set_network_proxy_settings(webkit2gtk::NetworkProxyMode::NoProxy, None);
    });
}

/// Adds the loopback hosts to `NO_PROXY`, preserving existing entries.
#[cfg(target_os = "linux")]
fn ensure_no_proxy() {
    let current = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    let mut hosts: Vec<String> = current
        .split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect();
    for host in ["localhost", "127.0.0.1", "::1"] {
        if !hosts.iter().any(|entry| entry == host) {
            hosts.push(host.to_string());
        }
    }
    let value = hosts.join(",");
    for key in ["NO_PROXY", "no_proxy"] {
        std::env::set_var(key, &value);
    }
}

fn has_env_proxy() -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|key| {
        std::env::var_os(key).is_some_and(|value| !value.is_empty())
    })
}

#[cfg(target_os = "linux")]
fn desktop_proxy() -> Option<String> {
    // GNOME, Unity, Cinnamon and other gsettings-based desktops. `auto` (PAC)
    // is not handled because evaluating a PAC script is out of scope.
    if gsettings("org.gnome.system.proxy", "mode").as_deref() != Some("manual") {
        return None;
    }
    let same_proxy = gsettings("org.gnome.system.proxy", "use-same-proxy").as_deref() == Some("true");
    let mut host = gsettings("org.gnome.system.proxy.https", "host");
    let mut port = gsettings("org.gnome.system.proxy.https", "port");
    if same_proxy || matches!(host.as_deref(), None | Some("")) {
        host = gsettings("org.gnome.system.proxy.http", "host");
        port = gsettings("org.gnome.system.proxy.http", "port");
    }
    build_proxy_url(host.as_deref(), port.as_deref())
}

#[cfg(target_os = "linux")]
fn gsettings(schema: &str, key: &str) -> Option<String> {
    let output = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    Some(value.trim().trim_matches('\'').trim().to_string())
}

fn build_proxy_url(host: Option<&str>, port: Option<&str>) -> Option<String> {
    let host = host?.trim();
    let port: u16 = port?.trim().parse().ok()?;
    if host.is_empty() || port == 0 {
        return None;
    }
    Some(format!("http://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::build_proxy_url;

    #[test]
    fn builds_http_proxy_url() {
        assert_eq!(
            build_proxy_url(Some("127.0.0.1"), Some("7897")),
            Some("http://127.0.0.1:7897".to_string())
        );
    }

    #[test]
    fn rejects_incomplete_settings() {
        assert_eq!(build_proxy_url(None, Some("7897")), None);
        assert_eq!(build_proxy_url(Some(""), Some("7897")), None);
        assert_eq!(build_proxy_url(Some("127.0.0.1"), None), None);
        assert_eq!(build_proxy_url(Some("127.0.0.1"), Some("0")), None);
        assert_eq!(build_proxy_url(Some("127.0.0.1"), Some("abc")), None);
    }
}