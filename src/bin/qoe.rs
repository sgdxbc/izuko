use std::time::Duration;

use serde::Deserialize;
use tokio::{process::Command, task::JoinSet, time::Instant};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-54-233-234-50.sa-east-1.compute.amazonaws.com";
    let cid = "bafybeiftyvcar3vh7zua3xakxkb2h5ppo4giu5f3rkpsqgcfh7n7axxnsa";

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
        .map(|line| serde_json::from_str(line))
        .collect::<Result<Vec<FindProvs>, _>>()?
        .into_iter()
        .filter_map(|find_provs| {
            if find_provs.Type != 4 {
                return None;
            }
            find_provs.Responses
        })
        .flatten();

    println!("{:?}", find_provs_responses.collect::<Vec<_>>());

    let mut sessions = JoinSet::new();
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
        ));
    }

    while let Some(result) = sessions.join_next().await {
        let (id, score) = result??;
        println!("* Providers {id} Score {score}")
    }

    Ok(())
}

async fn get_session(
    index: usize,
    ipfs_host: String,
    id: String,
    addrs: Option<Vec<String>>,
    cid: String,
) -> anyhow::Result<(String, f32)> {
    let Some(addrs) = addrs else {
        println!("! [{index:04}] No available address to {id}");
        return Ok((id, 0.));
    };
    if addrs.is_empty() {
        println!("! [{index:04}] No available address to {id}");
        return Ok((id, 0.));
    }

    println!("* [{index:04}] Initialize ephemeral IPFS peer");
    let status = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(
            format!("export IPFS_PATH=/tmp/ipfs-{index}")
                + "; ipfs init --profile server,randomports"
                + "; ipfs key rm old"
                + "; ipfs key rotate -o old"
                + " && ipfs config Routing.Type none"
                + &format!(
                    " && ipfs config Addresses.API /ip4/127.0.0.1/tcp/{}",
                    15001 + index
                )
                + &format!(
                    " && ipfs config Addresses.Gateway /ip4/127.0.0.1/tcp/{}",
                    18080 + index
                ),
        )
        .status()
        .await?;
    anyhow::ensure!(status.success());

    println!("* [{index:04}] Start IPFS daemon");
    let mut daemon = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs daemon"))
        .spawn()?;
    let daemon_task = tokio::spawn(async move { daemon.wait().await });

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
                "IPFS_PATH=/tmp/ipfs-{index} timeout -s SIGINT 100s ipfs get -o /dev/null {cid}"
            ))
            .status()
            .await?;
        if !status.success() {
            println!("! [{index:04}] Failed to finish download {cid} via {id}");
            break 'score 0.;
        }
        1. / start.elapsed().as_secs_f32()
    };

    println!("! [{index:04}] Shutdown ephemeral IPFS peer");
    let status = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs shutdown"))
        .status()
        .await?;
    anyhow::ensure!(status.success());
    daemon_task.await??;

    Ok((id, score))
}
