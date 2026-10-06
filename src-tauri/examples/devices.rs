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
    }
    Ok(())
}
