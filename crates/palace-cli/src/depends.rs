//! `palace depends <claim-id|"<date> C7"> --kg <kg> [--json]` (M4d item 6): the provenance
//! around one claim — rollups, edges, chain, events, compiles. Read-only; reported, never acted on.
use clap::Args;
use palace_store::Error;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct Options {
    /// `c_…`, `f_…`, `#f_…` or "<rollup date> C7".
    pub target: String,
    /// The KG; compiler.sqlite is read beside it.
    #[arg(long)]
    pub kg: PathBuf,
    #[arg(long)]
    pub json: bool,
}

/// Exit 0 reported, 1 unknown target, 2 broken.
pub fn run(o: Options) -> i32 {
    match report(&o) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => crate::improve::fail("palace depends", e),
    }
}

fn report(o: &Options) -> Result<String, Error> {
    let kg = crate::paths::input(&o.kg)?;
    let compiler = crate::paths::input(
        &kg.parent()
            .ok_or("KG parent missing")?
            .join("compiler.sqlite"),
    )?;
    let d = palace_store::depends::depends(&compiler, &kg, &o.target)?;
    if o.json {
        return Ok(serde_json::to_string_pretty(&d)?);
    }
    Ok(text(&d))
}

fn text(d: &Value) -> String {
    let c = &d["claim"];
    let head = format!(
        "DEPENDS {} {} | {} | {} status={} confidence={}",
        s(&c["id"]),
        s(&c["subject"]),
        s(&c["predicate"]),
        s(&c["object"]),
        s(&c["status"]),
        c["confidence"]
    );
    let mut lines = vec![
        head,
        section(d, "rollups", rollup),
        section(d, "edges", edge),
    ];
    lines.push(section(d, "predecessors", |l| {
        format!(
            "{} {} ({})",
            s(&l["from_number"]),
            s(&l["from"]),
            s(&l["status"])
        )
    }));
    lines.push(section(d, "successors", |l| {
        format!("{} {}", s(&l["to_number"]), s(&l["to"]))
    }));
    lines.push(section(d, "events", |e| {
        format!("{} {} by {}", s(&e["id"]), s(&e["verb"]), s(&e["by"]))
    }));
    lines.push(section(d, "compiles", |c| s(&c["operation"])));
    lines.join("\n")
}

fn rollup(r: &Value) -> String {
    format!(
        "{} {} ({})",
        s(&r["rollup"]),
        s(&r["number"]),
        s(&r["date"])
    )
}

fn edge(e: &Value) -> String {
    format!(
        "{} {} {} valid_to={}",
        s(&e["triple"]),
        s(&e["predicate"]),
        s(&e["link"]),
        s(&e["valid_to"])
    )
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("-").to_owned()
}

/// `<key>: item; item` (or `none`).
fn section(d: &Value, key: &str, item: impl Fn(&Value) -> String) -> String {
    let items: Vec<String> = d[key].as_array().into_iter().flatten().map(item).collect();
    format!(
        "{key}: {}",
        if items.is_empty() {
            "none".into()
        } else {
            items.join("; ")
        }
    )
}
