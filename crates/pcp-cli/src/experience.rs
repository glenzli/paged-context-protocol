//! Native producers can prepare evidence while PCP is offline, then deliver one
//! exact request at a later checkpoint. This command never configures permissions.
use anyhow::{Context, Result, ensure};
use pcp_client::experience::{Experience, outbox::ExperienceOutbox, receipts};
use serde::Deserialize;
use std::{io::Read, path::PathBuf};

fn input() -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "experience input exceeds 1 MiB");
    Ok(bytes)
}

pub(super) fn outbox(args: &mut impl Iterator<Item = String>) -> Result<ExperienceOutbox> {
    let root = args.next().context("requires private outbox directory")?;
    let identity = args.next().context("requires expected Store identity")?;
    let principal = args.next().context("requires expected principal")?;
    ensure!(args.next().is_none(), "unexpected experience arguments");
    ExperienceOutbox::open(PathBuf::from(root), identity, principal)
}

pub(super) fn local_command(
    command: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<bool> {
    match command {
        "experience-stage" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Submission {
                scope: String,
                title: String,
                experience: Experience,
            }
            let outbox = outbox(args)?;
            let record: Submission = serde_json::from_slice(&input()?)?;
            let key = outbox.stage(
                record
                    .experience
                    .into_candidate(record.scope, record.title)?,
            )?;
            super::print_json(&serde_json::json!({"status":"queued_locally","key":key}))?;
        }
        "experience-receipt" => {
            let kind = args.next().context("requires infer or dev-mesh")?;
            let receipt = match kind.as_str() {
                "infer" => {
                    ensure!(args.next().is_none(), "unexpected receipt argument");
                    receipts::infer_response(&input()?)?
                }
                "dev-mesh" => {
                    let locator = args
                        .next()
                        .context("requires the producer's archived receipt locator")?;
                    ensure!(args.next().is_none(), "unexpected receipt argument");
                    receipts::dev_mesh_commit(&input()?, locator)?
                }
                _ => anyhow::bail!("unknown receipt producer"),
            };
            super::print_json(&receipt)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
