fn main() -> anyhow::Result<()> {
    let devices = dreamwhisper_lib::devices::scan()?;
    println!("{} diktafonvolymer hittades", devices.len());
    for d in devices {
        println!(
            "{} {} · {} · {}",
            d.vendor,
            d.model,
            d.label,
            d.mount_path.as_deref().unwrap_or("inte monterad")
        );
        if let Some(mount) = d.mount_path {
            match dreamwhisper_lib::devices::recording_directories(std::path::Path::new(&mount)) {
                Ok(directories) => {
                    for directory in directories {
                        println!("Inspelningsmapp: {}", directory.display());
                    }
                }
                Err(e) => println!("Inspelningsmapp: {e:#}"),
            }
        }
    }
    Ok(())
}
