use std::{process::Command, thread::sleep, time::Duration};

fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-54-233-234-50.sa-east-1.compute.amazonaws.com";

    println!("* Generate random data and add to IPFS");
    let output = Command::new("ssh")
        .arg(ipfs_host)
        .arg("dd if=/dev/random bs=16M count=1 of=testdata && ipfs add testdata -Q")
        .output()?;
    if !output.status.success() {
        print!("{}", String::from_utf8(output.stderr)?);
        anyhow::bail!("{}", output.status)
    }
    let cid = String::from_utf8(output.stdout)?.trim().to_string();

    println!("* Wait for providing data {cid}");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg(format!("ipfs routing provide {cid}"))
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Clean up added data");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg(format!(
            // "ipfs pin rm {cid} && ipfs repo gc && ipfs shutdown"
            "ipfs pin rm {cid} && ipfs repo gc"
        ))
        .status()?;
    if !status.success() {
        anyhow::bail!("{status}")
    }

    println!("* Wait IPFS to propagate trace");
    sleep(Duration::from_secs(10));

    println!("* Retrieve trace");
    let status = Command::new("python3")
        .args(["retrieve-traces.py", ipfs_host, "IpfsDHT.Provide", "data.provide/traces.sae"])
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
