use crate::graph::{self, GraphPaths};
use clap::{Args, Subcommand};
use palace_core::{
    compile::{Task, compile},
    retire::retire_fixtures,
    stress::{self, StressInput},
    traits::GraphMutation,
    traverse::{Options, traverse},
};
use palace_store::{Error, graph::GraphReader};
#[derive(Args)]
pub struct ReadOptions {
    #[command(flatten)]
    pub paths: GraphPaths,
    pub entity: String,
    #[arg(long, default_value_t = 2)]
    pub max_hops: usize,
    #[arg(long, default_value_t = 800)]
    pub token_budget: usize,
    #[arg(long)]
    pub include_retired: bool,
}
impl ReadOptions {
    fn options(&self) -> Options {
        Options {
            max_hops: self.max_hops,
            token_budget: self.token_budget,
            include_retired: self.include_retired,
        }
    }
}
pub fn read(args: ReadOptions, task: Option<Task>) -> Result<(), Error> {
    let graph = GraphReader::open(&crate::paths::input(&args.paths.kg)?)?;
    let result = match task {
        Some(task) => compile(&graph, &args.entity, task, args.options())?,
        None => traverse(&graph, &args.entity, args.options())?,
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
pub fn retire(paths: GraphPaths, name: &str, reason: &str, by: &str) -> Result<(), Error> {
    println!("{}", graph::open(&paths)?.retire_entity(name, reason, by)?);
    Ok(())
}
pub fn fixtures(paths: GraphPaths, by: &str) -> Result<(), Error> {
    println!("{}", retire_fixtures(&mut graph::open(&paths)?, by)?);
    Ok(())
}
#[derive(Subcommand)]
pub enum StressCommand {
    Record {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        predicate: String,
        #[arg(long)]
        object: String,
        #[arg(long)]
        reviewer: String,
        #[arg(long)]
        verdict: String,
        #[arg(long)]
        note: String,
    },
    Gaps {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long, default_value_t = 2)]
        threshold: usize,
    },
}
pub fn stress(command: StressCommand) -> Result<(), Error> {
    match command {
        StressCommand::Record {
            paths,
            subject,
            predicate,
            object,
            reviewer,
            verdict,
            note,
        } => {
            let mut graph = graph::open(&paths)?;
            let now = graph.now()?;
            println!(
                "{}",
                stress::record_stress(
                    &mut graph,
                    &StressInput {
                        subject,
                        predicate,
                        object,
                        reviewer,
                        verdict,
                        note
                    },
                    &now
                )?
            );
        }
        StressCommand::Gaps { paths, threshold } => {
            let graph = GraphReader::open(&crate::paths::input(&paths.kg)?)?;
            println!(
                "{}",
                serde_json::to_string(&stress::load_bearing_gaps(&graph, threshold)?)?
            );
        }
    }
    Ok(())
}
