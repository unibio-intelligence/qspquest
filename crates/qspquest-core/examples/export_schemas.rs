use std::{env, fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || args.len() > 2 || (args.len() == 2 && args[1] != "--check") {
        return Err("usage: export_schemas OUTPUT_DIRECTORY [--check]".into());
    }
    let directory = PathBuf::from(&args[0]);
    let check = args.len() == 2;
    if !check {
        fs::create_dir_all(&directory)?;
    }
    for (name, schema) in qspquest_core::schema::documents() {
        let content = serde_json::to_string_pretty(&schema)? + "\n";
        let path = directory.join(name);
        if check {
            if fs::read_to_string(&path)? != content {
                return Err(format!("stale generated schema: {}", path.display()).into());
            }
        } else {
            fs::write(&path, content)?;
        }
        println!(
            "{} {}",
            if check { "verified" } else { "generated" },
            path.display()
        );
    }
    Ok(())
}
