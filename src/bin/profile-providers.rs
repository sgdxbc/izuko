use std::{
    fmt::Write,
    net::{Ipv4Addr, Ipv6Addr},
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use serde::Deserialize;
use tokio::{
    fs::{create_dir_all, read, read_dir, write},
    process::Command,
    task::JoinSet,
    time::{sleep, Instant},
};

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

    // let dag = false;
    let dag = true;

    #[allow(non_snake_case)]
    #[derive(Deserialize, Debug)]
    struct FindProvsResponse {
        Addrs: Vec<String>,
        ID: String,
    }

    let mut path = None;
    let mut read_dir = read_dir(format!("saved/dump-providers/{cid}")).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let entry_path = Some(entry.path().with_extension("").with_extension(""));
        path = path.max(entry_path)
    }
    let path = path.ok_or(anyhow::anyhow!("no dumped providers for {cid}"))?;
    let mut download_csv_content = Arc::new(Mutex::new(String::new()));

    let responses = serde_json::from_slice::<Vec<FindProvsResponse>>(
        &read(path.with_extension("json")).await?,
    )?;
    println!(
        "* Loaded {} provider records from {}",
        responses.len(),
        path.display()
    );
    let route_responses = serde_json::from_slice::<Vec<FindProvsResponse>>(
        &read(path.with_extension("route.json")).await?,
    )?;
    println!(
        "* Loaded {} provider records (explicit routing)",
        responses.len()
    );

    let mut sessions = JoinSet::new();
    let mut responses = responses
        .into_iter()
        .map(|find_provs| (find_provs, false))
        .chain(
            route_responses
                .into_iter()
                .map(|find_provs| (find_provs, true)),
        );
    for (index, (find_provs, route)) in responses.by_ref().take(10).enumerate() {
        sessions.spawn(get_session(
            index,
            ipfs_host.into(),
            find_provs.ID,
            find_provs.Addrs,
            cid.into(),
            dag,
            route,
            download_csv_content.clone(),
        ));
    }

    // this part feels a little silly = =
    let mut failed = false;
    let mut overall_result = Ok(());
    while let Some(result) = sessions.join_next().await {
        let index = match result.map_err(Into::into).and_then(|result| result) {
            Ok(index) => index,
            Err(err) => {
                println!("! {err}");
                failed = true;
                overall_result = Err(err);
                continue;
            }
        };
        if !failed {
            if let Some((find_provs, route)) = responses.next() {
                sessions.spawn(get_session(
                    index,
                    ipfs_host.into(),
                    find_provs.ID,
                    find_provs.Addrs,
                    cid.into(),
                    dag,
                    route,
                    download_csv_content.clone(),
                ));
            }
        }
    }
    overall_result?;

    println!("* Clean up IPFS directories");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("rm -rf /tmp/ipfs-*")
        .status()
        .await?;
    anyhow::ensure!(status.success());

    let path = format!(
        "saved/profile-providers/{cid}/{}.download.csv",
        UNIX_EPOCH.elapsed()?.as_millis()
    );
    let path = Path::new(&path);
    println!("* Save download metrics to {}", path.display());
    create_dir_all(path.parent().unwrap()).await?;
    let download_csv_content = Arc::get_mut(&mut download_csv_content)
        .ok_or(anyhow::anyhow!("unexpected reference"))?
        .get_mut()
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    write(path, download_csv_content).await?;
    Ok(())
}

async fn get_session(
    index: usize,
    ipfs_host: String,
    id: String,
    addrs: Vec<String>,
    cid: String,
    dag: bool,
    route: bool,
    download_csv_content: Arc<Mutex<String>>,
) -> anyhow::Result<usize> {
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
        println!("! [{index:02}] No available address to {id}");
        return Ok(index);
    }

    println!("* [{index:02}] Initialize ephemeral IPFS peer");
    let output = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(
            format!("export IPFS_PATH=/tmp/ipfs-{index}")
                + "; ipfs shutdown"
                + "; ipfs init --profile server,randomports"
                + "; ipfs key rm old"
                + "; ipfs pin ls -t recursive -q | ipfs pin rm"
                + "; ipfs key rotate -o old"
                + " && ipfs repo gc"
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
        .stdout(Stdio::null())
        .output()
        .await?;
    anyhow::ensure!(
        output.status.success(),
        "{:?}",
        String::from_utf8(output.stderr)
    );

    println!("* [{index:02}] Start IPFS daemon for downloading from {id}");
    let mut daemon = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs daemon"))
        .stdout(Stdio::null())
        .spawn()?;
    let daemon_session = tokio::spawn(async move { daemon.wait().await });
    println!("* [{index:02}] Wait for IPFS daemon up");
    while {
        sleep(Duration::from_millis(1000)).await;
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs stats bw"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;
        !status.success()
    } {}

    'job: {
        println!("* [{index:02}] Connect provider peer");
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!(
                "IPFS_PATH=/tmp/ipfs-{index} ipfs swarm connect {}",
                addrs.join(" ")
            ))
            .status()
            .await?;
        if !status.success() {
            println!("! [{index:02}] All attempts to connect {id} failed");
            break 'job;
        }

        println!("* [{index:02}] Download from peer");
        let start = Instant::now();
        let status = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!(
                // "IPFS_PATH=/tmp/ipfs-{index} timeout -s SIGINT 100s ipfs {} {cid} {}",
                "IPFS_PATH=/tmp/ipfs-{index} timeout -s SIGINT 30s ipfs {} {cid} {}",
                if dag { "dag get" } else { "get -o /dev/null" },
                if dag { " && echo" } else { "" }
            ))
            .status()
            .await?;
        if !status.success() {
            println!("! [{index:02}] Failed to finish download via {id}");
            break 'job;
        }

        {
            let mut download_csv_content = download_csv_content
                .lock()
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            writeln!(
                &mut download_csv_content,
                "{id},{},{route},{}",
                ipfs_host.split('.').nth(1).unwrap_or("unknown"),
                start.elapsed().as_secs_f32()
            )?
        }
    };

    println!("* [{index:02}] Shutdown ephemeral IPFS peer");
    let status = Command::new("ssh")
        .arg(&ipfs_host)
        .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs shutdown"))
        .status()
        .await?;
    anyhow::ensure!(status.success());
    daemon_session.await??;

    Ok(index)
}
