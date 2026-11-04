//! Host-authenticated diary filing through the MCP writer, without wing overrides.
use clap::Args;
use palace_store::{Error, privacy};
use std::{path::PathBuf, time::Duration};

#[derive(Args)]
pub struct Options {
    #[arg(long, conflicts_with = "conversation")]
    owner: bool,
    #[arg(long)]
    palace: PathBuf,
    /// Main diary root (the parent of diary/).
    #[arg(long)]
    diary_root: PathBuf,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    text: String,
    #[arg(long, default_value = "mempalace")]
    mempalace: PathBuf,
}

/// Resolve adapter authority before opening any output. An absent/unknown audience refuses.
pub fn marker(owner: bool) -> Result<Option<privacy::Marker>, Error> {
    let identity = privacy::identity();
    if owner {
        if identity.conversation.is_some() {
            return Err("--owner cannot be used with --conversation".into());
        }
        let owner = identity
            .people
            .as_ref()
            .ok_or("--owner requires people.json")?
            .unique_owner()?;
        Ok(Some(privacy::Marker {
            source: "owner-private:diary".into(),
            people: [owner.clone()].into(),
            audience: [owner].into(),
            private: true,
            quarantine: false,
        }))
    } else {
        Ok(identity
            .people
            .as_ref()
            .zip(identity.conversation.as_deref())
            .map(|(people, session)| people.resolve(session, None))
            .filter(|m| !m.quarantine && !m.people.is_empty()))
    }
}

pub fn run(o: Options) -> Result<(), Error> {
    let Some(marker) = marker(o.owner)? else {
        eprintln!("palace diary-write: unknown or missing conversation identity; nothing written");
        return Ok(());
    };
    let root = crate::paths::diary_root(&o.diary_root)?;
    // Check both write destinations, including symlinked diary directories and stores.
    crate::paths::database(&root.join("diary/.write-check"))?;
    let palace = crate::paths::input(&o.palace)?;
    crate::paths::database(&palace.join("compiler.sqlite"))?;
    let tool = palace_mcp::diary_tool::DiaryTool::new(palace_mcp::options::DiaryOptions {
        root,
        palace,
        wings: None,
        wing: palace_mempalace::mine::DEFAULT_WING.into(),
        agent: palace_mempalace::mine::DEFAULT_AGENT.into(),
        mempalace: o.mempalace,
        check: None,
        mine_timeout: Duration::from_secs(20),
    });
    println!(
        "{}",
        tool.call_main(&serde_json::json!({"title":o.title,"text":o.text}), marker)?
    );
    Ok(())
}
