//! Evaluate a compiler-captured project manifest without changing its sources.
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let manifest = args.next().ok_or_else(|| {
        anyhow::anyhow!(
            "Usage: project_oracle <manifest.json> <new-report.json> [--configured-program]"
        )
    })?;
    let output = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing new report path"))?;
    use bearwisdom::resolution_oracle::project::{evaluate_manifest_with_mode, ProjectBindingMode};
    let mode = match args.next() {
        None => ProjectBindingMode::Legacy,
        Some(flag) if flag == "--configured-program" => ProjectBindingMode::ConfiguredProgram,
        _ => anyhow::bail!("Unexpected argument"),
    };
    anyhow::ensure!(args.next().is_none(), "Unexpected arguments");
    anyhow::ensure!(
        !std::path::Path::new(&output).exists(),
        "Refusing to overwrite an existing report"
    );
    let report = evaluate_manifest_with_mode(std::path::Path::new(&manifest), mode)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    println!(
        "{}",
        serde_json::json!({"binding_mode":report.binding_mode,"compiler_calls":report.compiler_calls,"fresh":report.fresh.counts,"cold":report.cold.counts,
        "snapshot_changes":report.snapshot_changes.len(),"gate_eligible":report.gate_eligible})
    );
    Ok(())
}
