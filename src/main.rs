use std::{
    process::{Command, Stdio},
    thread::{sleep, spawn},
    time::Duration,
};

fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-3-1-209-56.ap-southeast-1.compute.amazonaws.com";
    let ipfs_canary_host = "ec2-18-142-162-160.ap-southeast-1.compute.amazonaws.com";

    println!("* Rotate canary identity");
    let status = Command::new("ssh")
        .arg(ipfs_canary_host)
        .arg("ipfs key rm old; ipfs key rotate -o old")
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Start canary daemon");
    let daemon_session = spawn(move || {
        Command::new("ssh")
            .arg(ipfs_canary_host)
            .arg("ipfs daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    });
    sleep(Duration::from_millis(1200));

    println!("* Generate random data and add to canary");
    let output = Command::new("ssh")
        .arg(ipfs_canary_host)
        .arg("dd if=/dev/random bs=1M count=1 of=testdata && ipfs add testdata -Q")
        .output()?;
    if !output.status.success() {
        print!("{}", String::from_utf8(output.stderr)?);
        anyhow::bail!("{}", output.status)
    }
    let cid = String::from_utf8(output.stdout)?.trim().to_string();

    println!("* Wait for providing data {cid}");
    let status = Command::new("ssh")
        .arg(ipfs_canary_host)
        .arg(format!(
            "ipfs routing provide {cid} && ipfs pin rm {cid} && ipfs repo gc && ipfs shutdown"
        ))
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Canary daemon shutdown");
    daemon_session.join().map_err(|err| {
        err.downcast::<anyhow::Error>()
            .map(|err| *err)
            .unwrap_or(anyhow::anyhow!("unknown error"))
    })??;

    println!("* Find provider for data {cid}");
    let output = Command::new("ssh")
        .arg(ipfs_host)
        .arg(format!("ipfs routing findprovs --num-providers=1 {cid}"))
        .output()?;
    if !output.status.success() {
        anyhow::bail!("{status}")
    }
    if String::from_utf8(output.stdout)?.trim().is_empty() {
        println!("! Find provider failed")
    }

    println!("* Wait IPFS to propagate trace");
    sleep(Duration::from_secs(10));

    println!("* Retrieve trace");
    let status = Command::new("python3")
        .args(["retrieve-traces.py", ipfs_host])
        .env(
            "PYTHONPATH",
            "./jaeger-idl/proto-gen-python:./jaeger-idl/proto-gen-python/github/com/gogo/protobuf/",
        )
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Restart telemetry collector");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("docker restart $(docker ps -q)")
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    Ok(())
}
