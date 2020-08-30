use serde::Deserialize;
use std::fs;
use tonic::{transport::Channel, Request};

use flo_grpc::controller::flo_controller_client::*;
use flo_grpc::controller::ImportMapChecksumsRequest;
use flo_grpc::game::MapChecksumImportItem;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut items = vec![];
  let dir = fs::read_dir("deps/wc3-samples/map_checksum")?;

  for entry in dir {
    let entry = entry?;
    let path = entry.path();
    let file = fs::File::open(path)?;
    let content: Content = serde_json::from_reader(file)?;
    items.push(MapChecksumImportItem {
      sha1: to_hex(content.sha1),
      checksum: content.checksum,
    });
  }

  let channel = Channel::from_static("tcp://service.w3flo.com:3549")
    .connect()
    .await?;
  let mut client = FloControllerClient::with_interceptor(channel, |mut req: Request<()>| {
    req
      .metadata_mut()
      .insert("x-flo-secret", "x9&hZx$2hG047GAT".parse().unwrap());
    Ok(req)
  });

  let res = client
    .import_map_checksums(ImportMapChecksumsRequest { items })
    .await?;

  dbg!(res);

  Ok(())
}

fn to_hex(sha1: Vec<u8>) -> String {
  sha1.iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Debug, Deserialize)]
struct Content {
  sha1: Vec<u8>,
  path: String,
  checksum: u32,
}
