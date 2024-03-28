use std::{
    fmt::Write,
    path::Path,
    process::Stdio,
    time::{Duration, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, write},
    process::Command,
    spawn,
    time::{sleep, Instant},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let ipfs_host = "ec2-3-1-209-56.ap-southeast-1.compute.amazonaws.com";

    // ipfs sigcomm'22
    // let cid = "bafybeiftyvcar3vh7zua3xakxkb2h5ppo4giu5f3rkpsqgcfh7n7axxnsa";
    // hello world DAG
    // let cid = "baguqeerasords4njcts6vs7qvdjfcvgnume4hqohf65zsfguprqphs3icwea";
    // hello
    let cid = "bafkreibm6jg3ux5qumhcn2b3flc3tyu6dmlb4xa7u5bf44yegnrjhc4yeq";
    // hello (blake3), appears no provider
    // let cid = "bafkr4ihkr4ld3m4gqkjf4reryxsy2s5tkbxprqkow6fin2iiyvreuzzab4";
    // apollo
    // let cid = "QmSnuWmxptJZdLJpKRarxBMS2Ju2oANVrgbr2xWbie9b2D";

    // let dag = true;

    println!("* Start IPFS daemon");
    let daemon_session = spawn(
        Command::new("ssh")
            .arg(ipfs_host)
            .arg("ipfs daemon")
            .stdout(Stdio::null())
            .status(),
    );
    sleep(Duration::from_millis(4200)).await;

    let result = async {
        #[allow(non_snake_case, unused)]
        #[derive(Deserialize, Debug)]
        struct FindProvs {
            Extra: String,
            ID: String,
            Responses: Option<Vec<FindProvsResponse>>,
            Type: i32,
        }

        #[allow(non_snake_case)]
        #[derive(Debug, Serialize, Deserialize)]
        struct FindProvsResponse {
            Addrs: Vec<String>,
            ID: String,
        }

        println!("* Find providers for {cid}");
        let find_provs = reqwest::Client::new()
            .post(format!("http://{ipfs_host}:5001/api/v0/routing/findprovs"))
            .query(&[("arg", cid), ("num-providers", "10000")])
            .timeout(Duration::from_secs(100))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let mut find_provs_responses = find_provs
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
        anyhow::ensure!(find_provs_responses.len() < 10000);

        // println!("{find_provs_responses:?}");

        let path = format!(
            "saved/dump-providers/{cid}/{}",
            UNIX_EPOCH.elapsed()?.as_millis()
        );
        println!(
            "* Dump {} provider records to {path}",
            find_provs_responses.len()
        );
        let path = Path::new(&path);
        create_dir_all(path.parent().unwrap()).await?;
        write(
            path.with_extension("json"),
            serde_json::to_vec_pretty(&find_provs_responses)?,
        )
        .await?;

        let mut route_csv_content = String::new();
        for response in &mut find_provs_responses {
            println!("* Find provider {}", response.ID);
            let start = Instant::now();
            let output = Command::new("ssh")
                .arg(ipfs_host)
                .arg(format!(
                    "timeout -s SIGINT 100s ipfs routing findpeer {}",
                    response.ID
                ))
                .output()
                .await?;
            if output.status.success() {
                let query_duration = start.elapsed();
                response.Addrs = String::from_utf8(output.stdout)?
                    .lines()
                    .map(|line| line.trim().into())
                    .collect();
                writeln!(
                    &mut route_csv_content,
                    "{},{},{}",
                    response.ID,
                    ipfs_host.split('.').nth(1).unwrap_or("unknown"),
                    query_duration.as_secs_f32()
                )?;
            } else {
                println!("! Provider {} not routable", response.ID);
                response.Addrs.clear()
            }
        }

        println!("* Dump provider records with explicit routing");
        write(
            path.with_extension("route.json"),
            serde_json::to_vec_pretty(&find_provs_responses)?,
        )
        .await?;
        write(path.with_extension("route.csv"), route_csv_content).await?;

        Ok(())
    }
    .await;
    if result.is_err() {
        println!("! Job failed")
    }

    println!("* Shutdown IPFS daemon");
    let status = Command::new("ssh")
        .arg(ipfs_host)
        .arg("ipfs shutdown")
        .status()
        .await?;
    anyhow::ensure!(status.success());
    daemon_session.await??;

    result
}
