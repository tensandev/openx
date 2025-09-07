use openx_arg0::arg0_dispatch_or_else;
use openx_common::CliConfigOverrides;
use openx_mcp_server::run_main;

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|openx_linux_sandbox_exe| async move {
        run_main(openx_linux_sandbox_exe, CliConfigOverrides::default()).await?;
        Ok(())
    })
}
