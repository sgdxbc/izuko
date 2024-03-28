use std::{
    net::{Ipv4Addr, Ipv6Addr},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde::Deserialize;
use tokio::{process::Command, sync::Semaphore, task::JoinSet, time::Instant};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-54-233-234-50.sa-east-1.compute.amazonaws.com";

    // ipfs sigcomm'22
    // let cid = "bafybeiftyvcar3vh7zua3xakxkb2h5ppo4giu5f3rkpsqgcfh7n7axxnsa";
    // hello world DAG
    let cid = "baguqeerasords4njcts6vs7qvdjfcvgnume4hqohf65zsfguprqphs3icwea";
    // hello
    // let cid = "bafkreibm6jg3ux5qumhcn2b3flc3tyu6dmlb4xa7u5bf44yegnrjhc4yeq";
    // hello (blake3), appears no provider
    // let cid = "bafkr4ihkr4ld3m4gqkjf4reryxsy2s5tkbxprqkow6fin2iiyvreuzzab4";
    // apollo
    // let cid = "QmSnuWmxptJZdLJpKRarxBMS2Ju2oANVrgbr2xWbie9b2D";
    let dag = true;

    #[allow(non_snake_case, unused)]
    #[derive(Deserialize, Debug)]
    struct FindProvs {
        Extra: String,
        ID: String,
        Responses: Option<Vec<FindProvsResponse>>,
        Type: i32,
    }

    #[allow(non_snake_case)]
    #[derive(Deserialize, Debug)]
    struct FindProvsResponse {
        Addrs: Option<Vec<String>>,
        ID: String,
    }

    println!("* Find providers for {cid}");
    let find_provs = reqwest::Client::new()
        .post(format!("http://{ipfs_host}:5001/api/v0/routing/findprovs"))
        .query(&[("arg", cid), ("num-providers", "1000")])
        .timeout(Duration::from_secs(100))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let find_provs_responses = find_provs
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<FindProvs>, _>>()?
        .into_iter()
        .filter_map(|find_provs| {
            if find_provs.Type != 4 {
                return None;
            }
            find_provs.Responses
        })
        .flatten()
        .collect::<Vec<_>>();
    anyhow::ensure!(find_provs_responses.len() < 1000);

    let output = Command::new("ssh")
        .arg(ipfs_host)
        .arg(format!("ipfs routing findprovs {cid}"))
        .output()
        .await?;
    anyhow::ensure!(output.status.success());
    // TODO

    // println!("{:?}", find_provs_responses.collect::<Vec<_>>());

    let mut sessions = JoinSet::new();
    let semaphore = Arc::new(Semaphore::new(10));
    for (index, find_provs) in find_provs_responses.into_iter().enumerate() {
        println!(
            "* [{index:04}] Spawn download session with peer id {}",
            find_provs.ID
        );
        sessions.spawn(get_session(
            index,
            ipfs_host.into(),
            find_provs.ID,
            find_provs.Addrs,
            cid.into(),
            dag,
            semaphore.clone(),
        ));
    }

    while let Some(result) = sessions.join_next().await {
        let (id, score) = match result {
            Ok(Ok((id, score))) => (id, score),
            result => {
                println!("! {result:?}");
                continue;
            }
        };
        println!("*** Provider {id} Score {score}")
    }

    Ok(())
}

async fn get_session(
    index: usize,
    ipfs_host: String,
    id: String,
    addrs: Option<Vec<String>>,
    cid: String,
    dag: bool,
    semaphore: Arc<Semaphore>,
) -> anyhow::Result<(String, f32)> {
    let _permit = semaphore.acquire().await?;
    let mut addrs = addrs.unwrap_or_default();
    let fallback_query = addrs.is_empty();

    println!("* [{index:04}] Initialize ephemeral IPFS peer");
    let output = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(
            format!("export IPFS_PATH=/tmp/ipfs-{index}")
                + "; ipfs shutdown"
                + "; ipfs init --profile server,randomports"
                + "; ipfs key rm old"
                + "; ipfs key rotate -o old"
                + if fallback_query {
                    ""
                } else {
                    " && ipfs config Routing.Type none"
                }
                + &format!(
                    " && ipfs config Addresses.API /ip4/127.0.0.1/tcp/{}",
                    15001 + index
                )
                + &format!(
                    " && ipfs config Addresses.Gateway /ip4/127.0.0.1/tcp/{}",
                    18080 + index
                ),
        )
        .stdout(Stdio::null())
        .output()
        .await?;
    anyhow::ensure!(
        output.status.success(),
        "{:?}",
        String::from_utf8(output.stderr)
    );

    println!("* [{index:04}] Start IPFS daemon");
    let mut daemon = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs daemon"))
        .stdout(Stdio::null())
        .spawn()?;
    let mut daemon_task = tokio::spawn(async move { daemon.wait().await });
    tokio::time::sleep(Duration::from_millis(4200)).await;

    let mut query_duration = Duration::ZERO;
    if fallback_query {
        println!("> [{index:04}] No address in provider record, fallback to expicit routing");
        let start = Instant::now();
        let output = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!(
                "IPFS_PATH=/tmp/ipfs-{index} ipfs routing findpeer {id}"
            ))
            .output()
            .await?;
        if output.status.success() {
            addrs = String::from_utf8(output.stdout)?
                .lines()
                .map(|line| line.trim().into())
                .collect();
        }
        query_duration = start.elapsed();

        println!("> [{index:04}] Shutdown ephemeral IPFS peer");
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs shutdown"))
            .status()
            .await?;
        anyhow::ensure!(status.success());
        daemon_task.await??;

        if addrs.is_empty() {
            println!("! [{index:04}] No available address to {id}");
            return Ok((id, 0.));
        }

        println!("> [{index:04}] Rotate key and disable DHT");
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(
                format!("export IPFS_PATH=/tmp/ipfs-{index}")
                    + "; ipfs key rm old"
                    + "; ipfs key rotate -o old"
                    + " && ipfs config Routing.Type none",
            )
            .status()
            .await?;
        anyhow::ensure!(status.success());

        println!("> [{index:04}] Restart IPFS daemon");
        let mut daemon = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs daemon"))
            .stdout(Stdio::null())
            .spawn()?;
        daemon_task = tokio::spawn(async move { daemon.wait().await });
        tokio::time::sleep(Duration::from_millis(4200)).await;
    }

    let addrs = addrs
        .into_iter()
        .filter_map(|addr| {
            if addr
                .strip_prefix("/ip4/")
                .and_then(|addr| addr.split_once('/'))
                .and_then(|(ip, _)| ip.parse::<Ipv4Addr>().ok())
                .map(|ip| ip.is_loopback() || ip.is_private())
                .unwrap_or(false)
                || addr
                    .strip_prefix("/ip6/")
                    .and_then(|addr| addr.split_once('/'))
                    .and_then(|(ip, _)| ip.parse::<Ipv6Addr>().ok())
                    .map(|ip| ip.is_loopback())
                    .unwrap_or(false)
            {
                return None;
            }
            Some(format!("{addr}/p2p/{id}"))
        })
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        println!("! [{index:04}] No available address to {id}");
        return Ok((id, 0.));
    }

    let score = 'score: {
        println!("* [{index:04}] Connect provider peer");
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!(
                "IPFS_PATH=/tmp/ipfs-{index} ipfs swarm connect {}",
                addrs.join(" ")
            ))
            .status()
            .await?;
        if !status.success() {
            println!("! [{index:04}] All attempts to connect {id} failed");
            break 'score 0.;
        }

        println!("* [{index:04}] Download from peer");
        let start = Instant::now();
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!(
                "IPFS_PATH=/tmp/ipfs-{index} timeout -s SIGINT 100s ipfs {} {cid} {}",
                if dag { "dag get" } else { "get -o /dev/null" },
                if dag { " && echo" } else { "" }
            ))
            .status()
            .await?;
        if !status.success() {
            println!("! [{index:04}] Failed to finish download {cid} via {id}");
            break 'score 0.;
        }
        1. / (start.elapsed() + query_duration).as_secs_f32()
    };

    println!("* [{index:04}] Shutdown ephemeral IPFS peer");
    let status = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs shutdown"))
        .status()
        .await?;
    anyhow::ensure!(status.success());
    daemon_task.await??;

    Ok((id, score))
}
