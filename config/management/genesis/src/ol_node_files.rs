use std::{fmt::Debug, fs, net::Ipv4Addr, path::PathBuf};

use crate::storage_helper::StorageHelper;
use anyhow::bail;
use diem_config::{
    config::OnDiskStorageConfig,
    config::SafetyRulesService,
    config::{DiscoveryMethod, NetworkConfig, NodeConfig, Peer, PeerRole, PeerSet, SecureBackend},
    config::{Identity, WaypointConfig},
    network_id::NetworkId,
};
use diem_crypto::{ed25519::Ed25519PublicKey, x25519::PublicKey};
use diem_global_constants::{FULLNODE_NETWORK_KEY, GENESIS_WAYPOINT, OWNER_ACCOUNT, VALIDATOR_NETWORK_KEY};
use diem_management::{config::ConfigPath, error::Error, secure_backend::ValidatorBackend};
use diem_types::{chain_id::ChainId, waypoint::Waypoint, account_address::AccountAddress};
use ol_types::account::ValConfigs;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;
use diem_secure_storage::{CryptoStorage, KVStorage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeType {
    Validator,
    ValidatorFullNode,
    PublicFullNode,
}

/// Prints the public information within a store
#[derive(Debug, StructOpt)]
pub struct Files {
    #[structopt(flatten)]
    config: ConfigPath,
    #[structopt(flatten)]
    backend: ValidatorBackend,
    #[structopt(long)]
    namespace: String,
    #[structopt(long)]
    github_org: String,
    #[structopt(long)]
    repo: String,
    #[structopt(long)]
    chain_id: u8,
    /// If specified, compares the internal state to that of a
    /// provided genesis. Note, that a waypont might diverge from
    /// the provided genesis after execution has begun.
    #[structopt(long)]
    data_path: PathBuf,
    #[structopt(long, verbatim_doc_comment)]
    genesis_path: Option<PathBuf>,
    #[structopt(long, verbatim_doc_comment)]
    fullnode_only: bool,
    #[structopt(long, verbatim_doc_comment)]
    waypoint: Option<Waypoint>,
    #[structopt(long, verbatim_doc_comment)]
    layout_path: Option<PathBuf>,
    #[structopt(long, verbatim_doc_comment)]
    val_ip_address: Option<Ipv4Addr>,
}

impl Files {
    pub fn execute(self) -> Result<NodeConfig, Error> {
        write_node_config_files(
            self.data_path,
            self.chain_id,
            &self.github_org,
            &self.repo,
            &self.namespace,
            &None,
            &self.fullnode_only,
            self.waypoint,
            &self.layout_path,
            self.val_ip_address,
        )
        .map_err(|e| {
            Error::ConfigError(format!(
                "Could not write node config files, message: {}",
                e.to_string()
            ))
        })
    }
}

pub fn write_node_config_files(
    output_dir: PathBuf,
    chain_id: u8,
    github_org: &str,
    repo: &str,
    namespace: &str,
    prebuilt_genesis: &Option<PathBuf>,
    fullnode_only: &bool,
    _way_opt: Option<Waypoint>,
    layout_path: &Option<PathBuf>,
    val_ip_address: Option<Ipv4Addr>,
    
) -> Result<NodeConfig, anyhow::Error> {
    // TODO: Do we need github token path with public repo?
    let github_token_path = output_dir.join("github_token.txt");
    let chain_id = ChainId::new(chain_id);

    let remote = format!(
        "backend=github;repository_owner={github_org};repository={repo};token={path};namespace={ns}",
        repo=&repo,
        github_org=&github_org,
        path=github_token_path.to_str().unwrap(),
        ns=&namespace
    );

    let storage_helper = StorageHelper::get_with_path(output_dir.clone());

    let (_genesis_path, _genesis_waypoint) = make_genesis_file(
        &output_dir,
        prebuilt_genesis,
        remote,
        layout_path,
        storage_helper,
        chain_id,
        namespace,
    )?;

    make_node_yaml(
      output_dir,
      val_ip_address,
      namespace,
      *fullnode_only
    )
}

fn get_default_keystore_helper(output_dir: PathBuf) -> StorageHelper {
  StorageHelper::get_with_path(output_dir.join("key_store.json"))
}
/// Make all the node configurations needed
pub fn make_node_yaml(
  output_dir: PathBuf,
  val_ip_address: Option<Ipv4Addr>,
  namespace: &str,
  fullnode_only: bool,
) -> Result<NodeConfig, anyhow::Error>{
    // get the key_store storage interface for disk.
    let storage_helper = get_default_keystore_helper(output_dir.clone());
    let s = storage_helper.storage(namespace.to_owned());
    let gw = s.get::<Waypoint>(GENESIS_WAYPOINT)?.value;
      // Get node configs template
    let config = if fullnode_only {
        let mut n = make_fullnode_cfg(output_dir.clone(), gw)?;
        write_yaml(output_dir.clone(), &mut n, NodeType::PublicFullNode)?;
        n
    } else {
        // fullnode configs, only used for rescuing a validator node that's out of validator set.
        let mut fullnode = make_fullnode_cfg(output_dir.clone(), gw)?;
        write_yaml(output_dir.clone(), &mut fullnode, NodeType::PublicFullNode)?;
        
        // vfn configs
        if let Some(ip_address) = val_ip_address {
          let mut vfn = make_vfn_cfg(
            output_dir.clone(),
            gw,
            ip_address,
            namespace,
          )?;
          write_yaml(output_dir.clone(), &mut vfn, NodeType::PublicFullNode)?;
        } else {
          bail!("VFN settings requires a val_ip_address");
        }

        // validator configs

        let mut n = make_validator_cfg(
            output_dir.clone(),
            namespace,
        )?;
        write_yaml(output_dir.clone(), &mut n, NodeType::Validator)?;
        n
    };
    Ok(config)
}


fn write_yaml(
    output_dir: PathBuf,
    config: &mut NodeConfig,
    role: NodeType,
) -> Result<(), anyhow::Error> {
    let filename = match role {
        NodeType::Validator => "validator.node.yaml",
        NodeType::ValidatorFullNode => "vfn.node.yaml",
        NodeType::PublicFullNode => "fullnode.node.yaml",
    };

    let yaml_path = output_dir.join(filename);
    fs::create_dir_all(&output_dir)?;
    config.save(&yaml_path)?;

    println!(
        "validator configurations initialized, file saved to: {:?}",
        &yaml_path
    );
    Ok(())
}

fn make_genesis_file(
    output_dir: &PathBuf,
    prebuilt_genesis: &Option<PathBuf>,
    remote: String,
    layout_path: &Option<PathBuf>,
    storage_helper: StorageHelper,
    chain_id: ChainId,
    namespace: &str,
) -> Result<(PathBuf, Waypoint), anyhow::Error> {
    let genesis_path = output_dir.join("genesis.blob");
    match prebuilt_genesis {
        Some(path) => {
            // TODO: insert waypoint
            let wp: Waypoint =
                fs::read_to_string(&path.parent().unwrap().join("genesis_waypoint.txt"))?
                    .parse()?;
            Ok((path.to_owned(), wp))
        }
        None => {
            // building a genesis file requires a set_layout path. The default is for genesis to use a local set_layout file. Once a genesis occurs, the canonical chain can store the genesis information to github repo for future verification and creating a genesis blob.
            let genesis_waypoint = match layout_path {
                Some(layout_path) => storage_helper
                    .build_genesis_with_layout(chain_id, &remote, &genesis_path, &layout_path)
                    .unwrap(),
                None => {
                    println!("attempting to get a set_layout file from the genesis repo");
                    storage_helper
                        .build_genesis_from_github(chain_id, &remote, &genesis_path)
                        .unwrap()
                }
            };

            // for genesis cases, need to insert the waypoint in the key_store.json
            storage_helper
                .insert_waypoint(&namespace, genesis_waypoint)
                .unwrap();

            Ok((genesis_path, genesis_waypoint))
            // way_opt = Some(genesis_waypoint);
        }
    }
}

fn make_validator_cfg(
    output_dir: PathBuf,
    namespace: &str,
) -> Result<NodeConfig, anyhow::Error> {
    // TODO: make the validator node have mutual authentication with VFN.
    // for that it will need to get the Peer object of the VFN after the identity has been created
    // by default the VFN identity is random.
    let mut disk_storage = OnDiskStorageConfig::default();
    disk_storage.set_data_dir(output_dir.clone());
    disk_storage.path = output_dir.clone().join("key_store.json");
    disk_storage.namespace = Some(namespace.to_owned());

    let mut c = default_for_validator()?;
    c.set_data_dir(output_dir.clone());
    // Note skip setting namepace for later.
    c.base.waypoint =
        WaypointConfig::FromStorage(SecureBackend::OnDiskStorage(disk_storage.clone()));

    // If validator configs set val network configs
    let mut network = NetworkConfig::network_with_id(NetworkId::Validator);

    // NOTE: Using configs as described in cluster tests:
    // testsuite/cluster-test/src/cluster_swarm/configs/validator.yaml
    network.discovery_method = DiscoveryMethod::Onchain;
    network.mutual_authentication = true;
    network.identity = Identity::from_storage(
        VALIDATOR_NETWORK_KEY.to_string(),
        OWNER_ACCOUNT.to_string(),
        SecureBackend::OnDiskStorage(disk_storage.clone()),
    );
    network.network_address_key_backend = Some(SecureBackend::OnDiskStorage(disk_storage.clone()));

    c.validator_network = Some(network.clone());

    // Consensus
    c.base.waypoint =
        WaypointConfig::FromStorage(SecureBackend::OnDiskStorage(disk_storage.clone()));

    c.execution.backend = SecureBackend::OnDiskStorage(disk_storage.clone());
    c.execution.genesis_file_location = output_dir.clone().join("genesis.blob");

    c.consensus.safety_rules.service = SafetyRulesService::Thread;
    c.consensus.safety_rules.backend = SecureBackend::OnDiskStorage(disk_storage.clone());

    c.storage.prune_window = Some(20_000);

    // VFN Settings of the FullNode

    let id_for_vfn_network = Identity::from_storage(
        FULLNODE_NETWORK_KEY.to_string(),
        OWNER_ACCOUNT.to_string(),
        SecureBackend::OnDiskStorage(disk_storage.clone()),
    );

    // TODO: clean this up using an iter
    let vfn_net = &mut c.full_node_networks[0];
    vfn_net.identity = id_for_vfn_network;
    c.full_node_networks = vec![vfn_net.to_owned()];
    
    // NOTE: Validator does not have public JSON RPC enabled. Only for localhost queries
    // this is set with the NodeConfig defaults.

    Ok(c)
}

/// Save node configs to files
pub fn save_node_yaml_files(_output_dir: PathBuf) {}

/// make the fullnode NodeConfig
pub fn make_fullnode_cfg(
    home_path: PathBuf,
    waypoint: Waypoint,
) -> Result<NodeConfig, anyhow::Error> {
    // TODO: how to add seed peers?

    let mut n = default_for_public_fullnode()?;
    n.set_data_dir(home_path);
    n.base.waypoint = WaypointConfig::FromConfig(waypoint);

    n.storage.prune_window = Some(20_000);

    // Write yaml
    Ok(n)

    // ///////// FULL NODE CONFIGS ////////
    // let mut fn_network = NetworkConfig::network_with_id(NetworkId::Public);

    // fn_network.seed_addrs = Seeds::new(genesis_path.clone())
    //     .get_network_peers_info()
    //     .expect("Could not get seed peers");

    // fn_network.discovery_method = DiscoveryMethod::Onchain;
    // fn_network.listen_address = "/ip4/0.0.0.0/tcp/6179".parse().unwrap();
    // fn_network.identity = Identity::from_storage(
    //     FULLNODE_NETWORK_KEY.to_string(),
    //     OWNER_ACCOUNT.to_string(),
    //     SecureBackend::OnDiskStorage(disk_storage.clone()),
    // );
    // config.full_node_networks = vec![fn_network];
}

/// make the fullnode NodeConfig
pub fn make_vfn_cfg(
    output_dir: PathBuf,
    waypoint: Waypoint,
    // validator_addr: PeerId,
    ip_address: Ipv4Addr,
    namespace: &str,
    // fn_net_pubkey: PublicKey,
) -> Result<NodeConfig, anyhow::Error> {
    let mut n = default_for_vfn()?;

    let storage_helper = get_default_keystore_helper(output_dir.clone());
    // Set base properties
    n.set_data_dir(output_dir);
    n.base.waypoint = WaypointConfig::FromConfig(waypoint);

    let storage = storage_helper.storage(namespace.to_string());

    // set the Validator as the Seed peer for the VFN network
    // TODO: The validator address, Is it a namespace or is it used for authentication?
    // let validator_addr: AccountAddress = storage.get::<String>(OWNER_ACCOUNT)?.value.parse()?;
    let val_vfn_net_pubkey = storage.get_public_key(FULLNODE_NETWORK_KEY)?.public_key;
    let seeds = make_vfn_peer_set(val_vfn_net_pubkey, ip_address)?;

    // update the template (instead of creating from default)
    let net = &mut n.full_node_networks[0];
    net.seeds = seeds;
    n.full_node_networks = vec![net.to_owned()];

    n.storage.prune_window = Some(20_000);

    Ok(n)
}

fn make_vfn_peer_set(val_vfn_net_pubkey: Ed25519PublicKey, ip_address: Ipv4Addr) -> Result<PeerSet, Error>{
    let bytes = val_vfn_net_pubkey.to_bytes();
    let validator_vfn_net_addr = AccountAddress::from_identity_public_key(bytes.into());

    // let fn_net_pubkey = validators_id_for_vfn_network.
    // create the vfn network info.
    let val_peer_data = validator_upstream_peer_data(ip_address, bytes.into())?;
    // The seed address for the VFN can only be the Validator's address.
    let mut seeds = PeerSet::default();
    seeds.insert(validator_vfn_net_addr, val_peer_data);
    Ok(seeds)
}

pub fn validator_upstream_peer_data(ip_address: Ipv4Addr, pubkey: PublicKey) -> Result<Peer, Error> {
    let role = PeerRole::Validator;
    let val_addr = ValConfigs::make_vfn_addr(&ip_address.to_string(), pubkey);
    let p = Peer::from_addrs(role, vec![val_addr]);
    Ok(p)
}

pub fn default_for_public_fullnode() -> Result<NodeConfig, anyhow::Error> {
    let path_str = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(path_str)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ol/util/node_templates/fullnode.node.yaml");

    let contents = fs::read_to_string(&path)?;
    let n: NodeConfig = serde_yaml::from_str(&contents)?;

    Ok(n)
}

pub fn default_for_vfn() -> Result<NodeConfig, anyhow::Error> {
    let path_str = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(path_str)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ol/util/node_templates/vfn.node.yaml");

    let contents = fs::read_to_string(&path)?;
    let n: NodeConfig = serde_yaml::from_str(&contents)?;

    Ok(n)
}

pub fn default_for_validator() -> Result<NodeConfig, anyhow::Error> {
    let path_str = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(path_str)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ol/util/node_templates/validator.node.yaml");

    let contents = fs::read_to_string(&path)?;
    let n: NodeConfig = serde_yaml::from_str(&contents)?;

    Ok(n)
}
