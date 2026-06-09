use anyhow::Result;
use std::{fs, io::Read, path::Path};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init(log_file: &Path) -> Result<()> {
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let file_appender = tracing_appender::rolling::never(
        log_file.parent().unwrap_or_else(|| Path::new(".")),
        log_file.file_name().and_then(|name| name.to_str()).unwrap_or("sugt.log"),
    );

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("sugt=info,tower_http=info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr).compact())
        .with(fmt::layer().with_writer(file_appender).json())
        .try_init();

    Ok(())
}

pub fn tail(log_file: &Path, lines: usize) -> Result<Vec<String>> {
    if !log_file.exists() {
        return Ok(Vec::new());
    }

    let mut file = fs::File::open(log_file)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let mut result: Vec<String> = content.lines().rev().take(lines).map(ToOwned::to_owned).collect();
    result.reverse();
    Ok(result)
}
