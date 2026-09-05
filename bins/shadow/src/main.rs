use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    info!("shadow mode: would evaluate liquidations without submit");
    info!(
        "protocols: kamino={} p0={} save={}",
        liq_kamino::KLEND_PROGRAM_ID_MAINNET,
        liq_project0::MARGINFI_PROGRAM_ID_MAINNET,
        liq_save::SAVE_PROGRAM_ID_MAINNET
    );
    Ok(())
}
