use std::{
    process::{Command, Stdio},
    thread::{sleep, spawn},
    time::Duration,
};

fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-3-1-209-56.ap-southeast-1.compute.amazonaws.com";
    let cid = "QmW8MwfuojKUT2VAVPFXtHa21jrvcAK5Sc3MYiWFo3RXFq";

    println!("* Rotate identity");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("ipfs key rm old; ipfs key rotate -o old")
        .status()?;
    anyhow::ensure!(status.success());

    println!("* Disable reproviding");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("ipfs config --json Experimental.StrategicProviding true")
        .status()?;
    anyhow::ensure!(status.success());

    println!("* Start IPFS daemon");
    let daemon_session = spawn(move || {
        Command::new("ssh")
            .arg(ipfs_host)
            .arg("OTEL_EXPORTER_OTLP_INSECURE=true OTEL_TRACES_EXPORTER=otlp ipfs daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    });
    println!("* Wait for bootstrapping finish");
    sleep(Duration::from_millis(42000));

    println!("* Download data {cid}");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg(format!(
            "timeout -s SIGINT 20s ipfs get -o /dev/null --progress=false {cid}"
        ))
        .status()?;
    if !status.success() {
        println!("! Fail to download")
    }

    println!("* Clean downloaded blocks");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("ipfs repo gc")
        .status()?;
    anyhow::ensure!(status.success());

    println!("* Wait IPFS to propagate trace");
    sleep(Duration::from_secs(10));

    println!("* Retrieve trace");
    let status = Command::new("python3")
        .args([
            "retrieve-traces.py",
            ipfs_host,
            "CoreAPI.UnixfsAPI.Get",
            "data.get-hot.apse/traces.sae",
        ])
        .env(
            "PYTHONPATH",
            "./jaeger-idl/proto-gen-python:./jaeger-idl/proto-gen-python/github/com/gogo/protobuf/",
        )
        .status()?;
    // anyhow::ensure!(status.success());
    if !status.success() {
        println!("! {status}")
    }

    println!("* IPFS daemon shutdown");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("ipfs repo gc && ipfs shutdown")
        .status()?;
    anyhow::ensure!(status.success());
    let status = daemon_session.join().map_err(|err| {
        err.downcast::<anyhow::Error>()
            .map(|err| *err)
            .unwrap_or(anyhow::anyhow!("unknown error"))
    })??;
    anyhow::ensure!(status.success());

    println!("* Restart telemetry collector");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("docker restart $(docker ps -q)")
        .status()?;
    anyhow::ensure!(status.success());

    Ok(())
}
