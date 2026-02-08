use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct ListSourcesCommand;

impl ListSourcesCommand {
    pub async fn run(self) -> Result<()> {
        println!("Available sources:");
        for name in crate::engine::list_sources()? {
            println!("  - {}", name);
        }
        Ok(())
    }
}
