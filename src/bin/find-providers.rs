use std::{
    fmt::Write,
    iter::repeat,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use tokio::{
    fs::{create_dir_all, write},
    process::Command,
    task::JoinSet,
    time::sleep,
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

    let mut find_csv_content = Arc::new(Mutex::new(String::new()));
    let mut sessions = JoinSet::new();
    let mut responses = repeat(()).take(100);
    for (index, ()) in responses.by_ref().take(10).enumerate() {
        sessions.spawn(find_session(
            index,
            ipfs_host.into(),
            cid.into(),
            find_csv_content.clone(),
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
        if !failed && responses.next().is_some() {
            sessions.spawn(find_session(
                index,
                ipfs_host.into(),
                cid.into(),
                find_csv_content.clone(),
            ));
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
        "saved/find-providers/{cid}/{}.csv",
        UNIX_EPOCH.elapsed()?.as_millis()
    );
    let path = Path::new(&path);
    println!("* Save find provider results to {}", path.display());
    create_dir_all(path.parent().unwrap()).await?;
    let find_csv_content = Arc::get_mut(&mut find_csv_content)
        .ok_or(anyhow::anyhow!("unexpected reference"))?
        .get_mut()
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    write(path, find_csv_content).await?;
    Ok(())
}

async fn find_session(
    index: usize,
    ipfs_host: String,
    cid: String,
    find_csv_content: Arc<Mutex<String>>,
) -> anyhow::Result<usize> {
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
                // + " && ipfs config Routing.Type none"
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

    println!("* [{index:02}] Start IPFS daemon");
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

    // 'job: {
    {
        let output = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!("IPFS_PATH=/tmp/ipfs-{index} ipfs id -f '<id>'",))
            .output()
            .await?;
        anyhow::ensure!(output.status.success());
        let id = String::from_utf8(output.stdout)?;

        println!("* [{index:02}] Find providers from {id}");
        let output = Command::new("ssh")
            .arg(&ipfs_host)
            .arg(format!(
                "IPFS_PATH=/tmp/ipfs-{index} timeout -s SIGINT 100s ipfs routing findprovs {cid}",
            ))
            .output()
            .await?;
        anyhow::ensure!(output.status.success());

        {
            let mut find_csv_content = find_csv_content
                .lock()
                .map_err(|err| anyhow::anyhow!("{err}"))?;
            for line in String::from_utf8(output.stdout)?.lines() {
                writeln!(
                    &mut find_csv_content,
                    "{id},{},{}",
                    ipfs_host.split('.').nth(1).unwrap_or("unknown"),
                    line.trim()
                )?
            }
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
