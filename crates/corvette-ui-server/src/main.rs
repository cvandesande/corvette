//! Launches nginx with the development routes required by the Corvette UI.

use std::{
    env, fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, id},
};

fn main() -> io::Result<()> {
    let site_address = env::var("LEPTOS_SITE_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let site_root =
        absolute_path(&env::var("LEPTOS_SITE_ROOT").unwrap_or_else(|_| "target/site".to_owned()))?;
    let frigate_address =
        env::var("FRIGATE_ADDRESS").unwrap_or_else(|_| "127.0.0.1:5000".to_owned());
    let go2rtc_address =
        env::var("GO2RTC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:11984".to_owned());
    let reload_port = env::var("LEPTOS_RELOAD_PORT").unwrap_or_else(|_| "8081".to_owned());
    let config_path = env::temp_dir().join(format!("corvette-nginx-{}.conf", id()));
    let pid_path = env::temp_dir().join(format!("corvette-nginx-{}.pid", id()));
    let shell_path = env::temp_dir().join(format!("corvette-app-shell-{}.html", id()));
    prepare_development_shell(&site_root, &shell_path, &reload_port)?;
    let config = nginx_config(
        &site_address,
        &site_root,
        &shell_path,
        &frigate_address,
        &go2rtc_address,
        &pid_path,
    );
    fs::write(&config_path, config)?;

    let error = Command::new("nginx")
        .args([
            "-e",
            "stderr",
            "-c",
            config_path.to_str().expect("temporary path is UTF-8"),
        ])
        .exec();
    Err(error)
}

fn prepare_development_shell(
    site_root: &Path,
    shell_path: &Path,
    reload_port: &str,
) -> io::Result<()> {
    let shell = fs::read_to_string(site_root.join("app.html"))?;
    let reload_script = format!(
        r#"<script>
const reloadSocket = new WebSocket(
  `${{location.protocol === "https:" ? "wss" : "ws"}}://${{location.hostname}}:{reload_port}/live_reload`,
);
reloadSocket.addEventListener("message", (event) => {{
  JSON.parse(event.data);
  location.reload();
}});
</script>
</body>"#,
    );
    let development_shell = shell.replace("</body>", &reload_script);
    fs::write(shell_path, development_shell)
}

fn absolute_path(path: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(env::current_dir()?.join(path))
}

fn nginx_config(
    site_address: &str,
    site_root: &Path,
    shell_path: &Path,
    frigate_address: &str,
    go2rtc_address: &str,
    pid_path: &Path,
) -> String {
    format!(
        r#"worker_processes 1;
daemon off;
pid "{pid_path}";
error_log stderr;

events {{}}

http {{
    access_log off;

    types {{
        text/css css;
        text/html html;
        text/javascript js;
        application/wasm wasm;
    }}

    map $http_upgrade $connection_upgrade {{
        default upgrade;
        '' close;
    }}

    server {{
        listen {site_address};
        root "{site_root}";

        location /api/ {{
            proxy_pass http://{frigate_address};
            proxy_http_version 1.1;
            proxy_set_header Host $host;
        }}

        location /clips/ {{
            proxy_pass http://{frigate_address};
            proxy_http_version 1.1;
            proxy_set_header Host $host;
        }}

        location = /go2rtc/api/ws {{
            proxy_pass http://{go2rtc_address}/api/ws;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection $connection_upgrade;
            proxy_set_header Host $host;
        }}

        location /go2rtc/ {{
            proxy_pass http://{go2rtc_address}/;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
        }}

        location / {{
            try_files $uri @application_shell;
        }}

        location @application_shell {{
            internal;
            root "{shell_root}";
            try_files /{shell_name} =404;
        }}
    }}
}}
"#,
        pid_path = pid_path.display(),
        shell_name = shell_path
            .file_name()
            .expect("development shell path has a file name")
            .to_string_lossy(),
        shell_root = shell_path
            .parent()
            .expect("development shell path has a parent")
            .display(),
        site_root = site_root.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_preserves_proxy_paths_and_static_fallback() {
        let config = nginx_config(
            "127.0.0.1:8080",
            Path::new("/tmp/corvette-site"),
            Path::new("/tmp/corvette-app-shell.html"),
            "127.0.0.1:5000",
            "127.0.0.1:11984",
            Path::new("/tmp/corvette-nginx.pid"),
        );

        assert!(config.contains("location /api/"));
        assert!(config.contains("location /clips/"));
        assert!(config.contains("location = /go2rtc/api/ws"));
        assert!(config.contains("proxy_pass http://127.0.0.1:11984/api/ws"));
        assert!(config.contains("try_files $uri @application_shell"));
    }
}
