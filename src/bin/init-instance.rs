use std::{env::args, process::Command};

fn main() -> anyhow::Result<()> {
    let host = args()
        .nth(1)
        .ok_or(anyhow::anyhow!("missing host argument"))?;
    let ipfs_artifact = "../kubo/ipfs";

    println!("* Sync IPFS artifact");
    let status = Command::new("rsync")
        .arg(ipfs_artifact)
        .arg(format!("{host}:"))
        .status()?;
    anyhow::ensure!(status.success());

    println!("* Install IPFS artifact and configure");
    let status = Command::new("ssh")
        .arg(&host)
        .arg(concat!(
            "sudo cp ipfs /usr/local/bin",
            " && ipfs init --profile server",
            " && ipfs config Internal.Bitswap.ProviderSearchDelay 0",
            " && sudo sysctl -w net.core.rmem_max=2500000",
            " && sudo sysctl -w net.core.wmem_max=2500000",
        ))
        .status()?;
    anyhow::ensure!(status.success());

    let telemetry = args().nth(2);
    if telemetry.as_deref() == Some("telemetry") {
        println!("* Start telemetry");
        let status = Command::new("ssh")
            .arg(&host)
            .arg(concat!(
                "docker run -d --rm --name jaeger",
                " -e COLLECTOR_OTLP_ENABLED=true",
                " -e COLLECTOR_ZIPKIN_HOST_PORT=:9411",
                " -p 5775:5775/udp",
                " -p 6831:6831/udp",
                " -p 6832:6832/udp",
                " -p 5778:5778",
                " -p 16685:16685",
                " -p 16686:16686",
                " -p 14250:14250",
                " -p 14268:14268",
                " -p 14269:14269",
                " -p 4317:4317",
                " -p 4318:4318",
                " -p 9411:9411",
                " jaegertracing/all-in-one"
            ))
            .status()?;
        anyhow::ensure!(status.success());
    }

    Ok(())
}
