use std::{process::Stdio, time::Duration};

use serde::Deserialize;
use tokio::{process::Command, spawn, time::sleep};

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
        #[derive(Deserialize, Debug)]
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
        anyhow::ensure!(find_provs_responses.len() < 10000);

        println!("{find_provs_responses:?}");
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
