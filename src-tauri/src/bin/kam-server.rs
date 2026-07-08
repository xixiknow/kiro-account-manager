#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "console"
)]
#![allow(dead_code, unused_imports)]

#[path = "../auth/mod.rs"]
mod auth;
#[path = "../clients/mod.rs"]
mod clients;
#[path = "../commands/mod.rs"]
mod commands;
#[path = "../core/mod.rs"]
mod core;
#[path = "../gateway/mod.rs"]
mod gateway;
#[path = "../kiro/mod.rs"]
mod kiro;
#[path = "../models/mod.rs"]
mod models;
#[path = "../server_admin.rs"]
mod server_admin;
#[path = "../server_web.rs"]
mod server_web;
#[path = "../services/mod.rs"]
mod services;
#[path = "../state.rs"]
mod state;
#[path = "../tasks/mod.rs"]
mod tasks;
#[path = "../utils/mod.rs"]
mod utils;

#[derive(Debug, Default)]
struct ServerOptions {
    host: Option<String>,
    port: Option<u16>,
    client_api_keys: Vec<String>,
    prompt_cache_target_percent: Option<u16>,
    show_help: bool,
}

struct ServerLogger;

static SERVER_LOGGER: ServerLogger = ServerLogger;

impl log::Log for ServerLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level().to_level_filter() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        eprintln!(
            "{} {} [{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            record.level(),
            record
                .target()
                .rsplit("::")
                .next()
                .unwrap_or(record.target()),
            record.args()
        );
    }

    fn flush(&self) {}
}

fn setup_logger() {
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(log::LevelFilter::Trace),
            "debug" => Some(log::LevelFilter::Debug),
            "warn" => Some(log::LevelFilter::Warn),
            "error" => Some(log::LevelFilter::Error),
            "off" => Some(log::LevelFilter::Off),
            _ => Some(log::LevelFilter::Info),
        })
        .unwrap_or(log::LevelFilter::Info);

    if log::set_logger(&SERVER_LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

fn print_help() {
    println!(
        "Kiro Account Manager server\n\
         Usage: kam-server [options]\n\n\
         Options:\n\
           --host <host>                      Override KAM_HOST\n\
           --port <port>                      Override KAM_PORT\n\
           --client-api-key <key>             Add a Gateway client API key\n\
           --prompt-cache-target-percent <n>  Set Prompt Cache target percent (0-100)\n\
           -h, --help                         Print this help\n\n\
         Required env:\n\
           KAM_ADMIN_TOKEN                    Admin UI/API token\n"
    );
}

fn read_arg_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_port(value: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("port must be 1-65535: {value}"))?;
    if port == 0 {
        return Err("port must be greater than 0".to_string());
    }
    Ok(port)
}

fn parse_percent(value: &str, flag: &str) -> Result<u16, String> {
    let percent = value
        .parse::<u16>()
        .map_err(|_| format!("{flag} must be an integer from 0 to 100: {value}"))?;
    if percent > 100 {
        return Err(format!("{flag} must be from 0 to 100: {value}"));
    }
    Ok(percent)
}

fn parse_options() -> Result<ServerOptions, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut options = ServerOptions::default();
    let mut index = 0usize;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-h" | "--help" => options.show_help = true,
            "--host" => options.host = Some(read_arg_value(&args, &mut index, "--host")?),
            "--port" => {
                let value = read_arg_value(&args, &mut index, "--port")?;
                options.port = Some(parse_port(&value)?);
            }
            "--client-api-key" | "--api-key" => {
                options
                    .client_api_keys
                    .push(read_arg_value(&args, &mut index, arg)?);
            }
            "--prompt-cache-target-percent" | "--prompt-cache-hit-percent" => {
                let value = read_arg_value(&args, &mut index, arg)?;
                options.prompt_cache_target_percent = Some(parse_percent(&value, arg)?);
            }
            value if value.starts_with("--host=") => {
                options.host = Some(value["--host=".len()..].to_string());
            }
            value if value.starts_with("--port=") => {
                options.port = Some(parse_port(&value["--port=".len()..])?);
            }
            value if value.starts_with("--client-api-key=") => {
                options
                    .client_api_keys
                    .push(value["--client-api-key=".len()..].to_string());
            }
            value if value.starts_with("--api-key=") => {
                options
                    .client_api_keys
                    .push(value["--api-key=".len()..].to_string());
            }
            value if value.starts_with("--prompt-cache-target-percent=") => {
                options.prompt_cache_target_percent = Some(parse_percent(
                    &value["--prompt-cache-target-percent=".len()..],
                    "--prompt-cache-target-percent",
                )?);
            }
            value if value.starts_with("--prompt-cache-hit-percent=") => {
                options.prompt_cache_target_percent = Some(parse_percent(
                    &value["--prompt-cache-hit-percent=".len()..],
                    "--prompt-cache-hit-percent",
                )?);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }

    Ok(options)
}

fn split_csv_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn apply_env_and_options(
    config: &mut gateway::GatewayConfig,
    options: &ServerOptions,
) -> Result<(), String> {
    config.enabled = true;
    config.local_only = false;

    if let Ok(host) = std::env::var("KAM_HOST") {
        if !host.trim().is_empty() {
            config.host = host.trim().to_string();
        }
    }
    if let Ok(port) = std::env::var("KAM_PORT") {
        config.port = parse_port(port.trim())?;
    }
    for key in split_csv_env("KAM_CLIENT_API_KEYS") {
        if !config.client_api_keys.iter().any(|item| item == &key) {
            config.client_api_keys.push(key);
        }
    }
    let allowed_ips = split_csv_env("KAM_ALLOWED_IPS");
    if !allowed_ips.is_empty() {
        config.allowed_ips = allowed_ips;
    }
    if let Ok(percent) = std::env::var("KAM_PROMPT_CACHE_TARGET_PERCENT") {
        config.prompt_cache_target_percent =
            parse_percent(percent.trim(), "KAM_PROMPT_CACHE_TARGET_PERCENT")?;
    }

    if let Some(host) = options
        .host
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        config.host = host.to_string();
    }
    if let Some(port) = options.port {
        config.port = port;
    }
    for key in options
        .client_api_keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
    {
        if !config.client_api_keys.iter().any(|item| item == key) {
            config.client_api_keys.push(key.to_string());
        }
    }
    if let Some(percent) = options.prompt_cache_target_percent {
        config.prompt_cache_target_percent = percent;
    }

    if !config.local_only && config.allowed_ips.is_empty() {
        config.allowed_ips = vec![
            "127.0.0.1".to_string(),
            "::1".to_string(),
            "172.16.0.0/12".to_string(),
            "10.0.0.0/8".to_string(),
            "192.168.0.0/16".to_string(),
        ];
    }

    config.access_token = config.client_api_keys.first().cloned();
    Ok(())
}

#[tokio::main]
async fn main() {
    setup_logger();

    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("kam-server argument error: {error}");
            std::process::exit(2);
        }
    };

    if options.show_help {
        print_help();
        return;
    }

    if let Err(error) = server_admin::require_admin_token_configured() {
        eprintln!("{error}");
        std::process::exit(2);
    }

    let mut config = match gateway::load_gateway_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("failed to load gateway config: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = apply_env_and_options(&mut config, &options) {
        eprintln!("failed to apply server config: {error}");
        std::process::exit(2);
    }

    let mut runtime = match gateway::start_gateway_runtime(config.clone()).await {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start kam-server: {error}");
            std::process::exit(1);
        }
    };

    println!(
        "kam-server is running at http://{}:{}",
        config.host, config.port
    );
    println!("admin UI: http://{}:{}/", config.host, config.port);
    println!("health: http://{}:{}/healthz", config.host, config.port);

    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to wait for shutdown signal: {error}");
    }

    gateway::stop_gateway_runtime(&mut runtime).await;
    println!("kam-server stopped.");
}
