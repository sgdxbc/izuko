use std::{
    process::{Command, Stdio},
    thread::{sleep, spawn},
    time::Duration,
};

fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-3-1-209-56.ap-southeast-1.compute.amazonaws.com";
    let ipfs_canary_host = "nat-canary";

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
    sleep(Duration::from_millis(4200));

    println!("* Generate random data and add to canary");
    let output = Command::new("ssh")
        .arg(ipfs_canary_host)
        .arg("dd if=/dev/random bs=16M count=1 of=testdata && ipfs add testdata -Q")
        .output()?;
    if !output.status.success() {
        print!("{}", String::from_utf8(output.stderr)?);
        anyhow::bail!("{}", output.status)
    }
    let cid = String::from_utf8(output.stdout)?.trim().to_string();

    println!("* Wait for providing data {cid}");
    let status = Command::new("ssh")
        .arg(ipfs_canary_host)
        .arg(format!("ipfs routing provide {cid}"))
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Download data {cid}");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg(format!(
            "timeout -s SIGINT 100s ipfs get -o /dev/null --progress=false {cid}"
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
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Canary daemon shutdown");
    let status = Command::new("ssh")
        .arg(ipfs_canary_host)
        .arg(format!(
            "ipfs pin rm {cid} && ipfs repo gc && ipfs shutdown"
            // "ipfs pin rm {cid} && ipfs repo gc"
        ))
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }
    daemon_session.join().map_err(|err| {
        err.downcast::<anyhow::Error>()
            .map(|err| *err)
            .unwrap_or(anyhow::anyhow!("unknown error"))
    })??;

    println!("* Wait IPFS to propagate trace");
    sleep(Duration::from_secs(10));

    println!("* Retrieve trace");
    let status = Command::new("python3")
        .args(["retrieve-traces.py", ipfs_host, "CoreAPI.UnixfsAPI.Get"])
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
