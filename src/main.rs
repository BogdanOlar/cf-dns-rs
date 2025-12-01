extern crate serde_json;
use dotenv::dotenv;
use log::{error, info};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Display;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::time::Duration;
use std::{env, thread};

/// DNS record with the minimal obligatory fields required by Cloudflare
///
/// The record type is derived from the [`Record::content`] member by the [`Record::rtype()`] method.
#[derive(Debug, Clone)]
struct Record {
    name: String,
    ttl: Ttl,
    content: IpAddr,
    proxied: bool,
}

impl Record {
    fn rtype(&self) -> RecordType {
        RecordType::from_ip(&self.content)
    }
}

/// Cloudflare DNS record with just the fields needed for this app
#[derive(Debug, Clone)]
struct CfRecord {
    id: String,
    record: Record,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum RecordType {
    A,
    AAAA,
}

impl RecordType {
    fn from_ip(ip: &IpAddr) -> Self {
        match ip {
            IpAddr::V4(_) => Self::A,
            IpAddr::V6(_) => Self::AAAA,
        }
    }
}

impl TryFrom<&str> for RecordType {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "A" => Ok(Self::A),
            "AAAA" => Ok(Self::AAAA),
            _ => Err(()),
        }
    }
}

impl Display for RecordType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordType::A => write!(f, "A"),
            RecordType::AAAA => write!(f, "AAAA"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Ttl {
    Auto,
    Seconds(u32),
}

impl TryFrom<u32> for Ttl {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Auto),
            60..=86400 => Ok(Self::Seconds(value)),
            // Invalid value
            _ => Err(()),
        }
    }
}

impl From<Ttl> for u32 {
    fn from(value: Ttl) -> Self {
        match value {
            Ttl::Auto => 1,
            Ttl::Seconds(v) => v,
        }
    }
}

impl Default for Ttl {
    fn default() -> Self {
        Self::Auto
    }
}

impl Display for Ttl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ttl::Auto => write!(f, "1"),
            Ttl::Seconds(s) => write!(f, "{s}"),
        }
    }
}

fn get_external_ip(rtype: &RecordType, api_endpoint: &str) -> Result<IpAddr, ()> {
    let res = match reqwest::blocking::Client::new().get(api_endpoint).send() {
        Ok(r) => r,
        Err(e) => {
            error!("Could not get external IP from endpoint '{api_endpoint}': {e}");
            return Err(());
        }
    };

    if res.status().is_success() {
        let body = match res.text() {
            Ok(b) => b,
            Err(e) => {
                error!("Could not get external IP from endpoint '{api_endpoint}' response: {e}");
                return Err(());
            }
        };

        match rtype {
            RecordType::A => match Ipv4Addr::from_str(&body) {
                Ok(ip) => Ok(IpAddr::V4(ip)),
                Err(e) => {
                    error!(
                        "Could not parse IPv4 '{body}' from endpoint '{api_endpoint}' response: {e}"
                    );
                    Err(())
                }
            },
            RecordType::AAAA => match Ipv6Addr::from_str(&body) {
                Ok(ip) => Ok(IpAddr::V6(ip)),
                Err(e) => {
                    error!(
                        "Could not parse IPv6 '{body}' from endpoint '{api_endpoint}' response: {e}"
                    );
                    Err(())
                }
            },
        }
    } else {
        error!(
            "Could not connect to IP API endpoint: {}",
            res.error_for_status().unwrap_err()
        );
        Err(())
    }
}

fn cf_update_record_ip(
    zone_id: &str,
    record_id: &str,
    ip: &IpAddr,
    api_token: &str,
) -> Result<(), ()> {
    let client = reqwest::blocking::Client::new();

    let url = format!(
        "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
        zone_id, record_id
    );

    let body = json!({
        "content": ip,
    });

    let res = client
        .patch(&url)
        .header("Authorization", format!("Bearer {}", api_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .map_err(|_| ())?;

    if res.status().is_success() {
        Ok(())
    } else {
        error!(
            "Failed to update record: {}",
            res.text().unwrap_or("Unknown".to_string())
        );
        Err(())
    }
}

fn cf_create_record(record: &Record, zone_id: &str, api_token: &str) -> Result<(), ()> {
    let client = reqwest::blocking::Client::new();
    let post_url = format!("https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records");

    let body = serde_json::json!({
        "name": record.name,
        "type": record.rtype().to_string(),
        "content": record.content.to_string(),
        "ttl": u32::from(record.ttl) ,
        "proxied": record.proxied
    });

    let res = client
        .post(&post_url)
        .header("Authorization", format!("Bearer {}", api_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| {
            error!(
                "Could not create DNS record for host '{}' with ip '{}': {}",
                record.name, record.content, e
            );
            ()
        })?;

    if res.status().is_success() {
        Ok(())
    } else {
        match res.text() {
            Ok(text) => error!(
                "Failed to create DNS record for host '{}' with ip '{}': {}",
                record.name, record.content, text
            ),
            Err(e) => error!(
                "Failed to create DNS record for host '{}' with ip '{}': {}",
                record.name, record.content, e
            ),
        }
        error!("\tRequest URL: {post_url}");
        error!("\tRequest body: {body}");
        Err(())
    }
}

/// Get all DNS records of type `A` and `AAAA`
fn cf_get_records(zone_id: &str, api_token: &str) -> Result<Vec<CfRecord>, ()> {
    let client = reqwest::blocking::Client::new();

    let url = format!(
        "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
        zone_id,
    );

    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_token))
        .header("Content-Type", "application/json")
        .send()
        .map_err(|e| {
            error!("Could not get DNS records: {}", e);
            ()
        })?;

    let json = match res.json::<serde_json::Value>() {
        Ok(v) => v,
        Err(e) => {
            error!("Could not parse DNS records: {}", e);
            return Err(());
        }
    };

    let json_records = match json["result"].as_array() {
        Some(arr) => arr,
        None => {
            error!("Could not parse array of DNS records");
            return Err(());
        }
    };

    Ok(json_records
        .iter()
        .map(|value| cf_parse_record(value))
        .filter_map(|rec_res| rec_res.ok())
        .collect())
}

/// Try to parse a DNS record of type `A` and `AAAA`
fn cf_parse_record(value: &Value) -> Result<CfRecord, ()> {
    let id = value.get("id").ok_or(())?.as_str().ok_or(())?.to_string();
    let rtype = value.get("type").ok_or(())?.as_str().ok_or(())?;

    // Bail if the record type is not recognized as either `A` or `AAAA`
    let rtype = RecordType::try_from(rtype)?;

    let name = value.get("name").ok_or(())?.as_str().ok_or(())?.to_string();
    let ttl = value.get("ttl").ok_or(())?.as_u64().ok_or(())? as u32;
    let ttl: Ttl = ttl.try_into().map_err(|_| {
        error!("Record '{name}' with id '{id}': could not parse TTL value '{ttl}'");
        ()
    })?;

    let content = value.get("content").ok_or(())?.as_str().ok_or(())?;
    let content = match rtype {
        RecordType::A => IpAddr::V4(Ipv4Addr::from_str(&content).map_err(|e| {
            error!("Record '{name}' with id '{id}' of type '{rtype}': could not parse IPv4 value '{content}': {e}");
            ()
        })?),
        RecordType::AAAA => IpAddr::V6(Ipv6Addr::from_str(&content).map_err(|e| {
            error!("Record '{name}' with id '{id}' of type '{rtype}': could not parse IPv6 value '{content}': {e}");
            ()
        })?),
    };
    let proxied = value.get("proxied").ok_or(())?.as_bool().ok_or(())?;

    Ok(CfRecord {
        id,
        record: Record {
            name,
            ttl,
            content,
            proxied,
        },
    })
}

fn main() -> Result<(), ()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    dotenv().ok();

    let zone_id = env::var("CF_DNS_ZONE_ID").expect("CF_DNS_ZONE_ID not set");
    let api_token = env::var("CF_DNS_API_TOKEN").expect("CF_DNS_API_TOKEN not set");
    let hosts_string = env::var("CF_DNS_HOSTS").expect("CF_DNS_HOSTS not set");
    let hosts = hosts_string
        .trim()
        .split(";")
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();

    let ipv4_endpoint = env::var("IPV4_ENDPOINT").ok();
    let ipv6_endpoint = env::var("IPV6_ENDPOINT").ok();
    let mut endpoints = BTreeMap::new();
    if let Some(endpoint) = ipv4_endpoint {
        endpoints.insert(RecordType::A, endpoint);
    }
    if let Some(endpoint) = ipv6_endpoint {
        endpoints.insert(RecordType::AAAA, endpoint);
    }
    if endpoints.is_empty() {
        error!("At least one IP API endpoint must be defined!");
        return Err(());
    }

    let repeat_interval: u64 = env::var("REPEAT_INTERVAL_SECONDS")
        .unwrap_or("0".to_string()).parse().expect("Could not parse the value of `REPEAT_INTERVAL_SECONDS`. Make sure it is an unsigned value in the form `REPEAT_INTERVAL_SECONDS=60`");

    let create_records_allowed = env::var("CF_DNS_CREATE_HOST_RECORDS")
        .unwrap_or("false".to_string())
        .parse()
        .expect(
            "Could not read `CF_DNS_CREATE_HOST_RECORDS` which sould be either `true` or `false`",
        );

    // Print configuration info
    info!("Monitoring {} hosts:", hosts.len());
    for host in &hosts {
        info!("\t{host}");
    }
    info!("For DNS {} record types:", endpoints.keys().len());
    for key in endpoints.keys() {
        info!("\t{key}");
    }

    let mut cur_ips = BTreeMap::new();
    let mut prev_ips = BTreeMap::new();

    loop {
        // get current IPs
        for (rtype, endpoint) in &endpoints {
            if let Ok(ip) = get_external_ip(rtype, endpoint) {
                cur_ips.insert(*rtype, ip);
            }
        }

        if let Ok(cf_recs) = cf_get_records(&zone_id, &api_token) {
            for (rtype, cur_ip) in &cur_ips {
                let ip_label = match rtype {
                    RecordType::A => "IPv4",
                    RecordType::AAAA => "IPv6",
                };

                // Check IP change
                match prev_ips.get(rtype) {
                    Some(prev_ip) => {
                        if cur_ip != prev_ip {
                            info!("{ip_label} changed from '{prev_ip}' to '{cur_ip}'");
                        }
                    }
                    None => {
                        info!("{ip_label} changed from 'None' to '{cur_ip}'");
                    }
                }

                // Check and update DNS records
                for host in &hosts {
                    match cf_recs
                        .iter()
                        .find(|r| (r.record.name.as_str() == *host) && (r.record.rtype() == *rtype))
                    {
                        Some(cf_rec) => {
                            if cf_rec.record.content != *cur_ip {
                                match cf_update_record_ip(
                                    &zone_id,
                                    cf_rec.id.as_str(),
                                    cur_ip,
                                    &api_token,
                                ) {
                                    Ok(_) => info!(
                                        "Updated '{}' record '{}' from IP '{}' to '{}'",
                                        cf_rec.record.rtype(),
                                        cf_rec.record.name,
                                        cf_rec.record.content,
                                        cur_ip
                                    ),
                                    Err(_) => error!(
                                        "Failed to update '{}' record '{}' from IP '{}' to '{}'",
                                        cf_rec.record.rtype(),
                                        cf_rec.record.name,
                                        cf_rec.record.content,
                                        cur_ip
                                    ),
                                }
                            } else {
                                // Nothing to update, IPs are identical
                            }
                        }
                        None => {
                            if create_records_allowed {
                                let record = Record {
                                    name: (*host).to_string(),
                                    ttl: Ttl::default(),
                                    content: *cur_ip,
                                    proxied: false,
                                };

                                match cf_create_record(&record, &zone_id, &api_token) {
                                    Ok(_) => info!(
                                        "Created '{}' record '{}' with IP '{}'",
                                        *rtype, *host, cur_ip
                                    ),
                                    Err(_) => error!(
                                        "Failed to create '{}' record '{}' with IP '{}'",
                                        *rtype, *host, cur_ip
                                    ),
                                }
                            } else {
                                error!(
                                    "No cloudlflare record found with name '{}' of type '{}'",
                                    *host, *rtype
                                );
                            }
                        }
                    }
                }
            }
        }

        if repeat_interval > 0 {
            let temp = prev_ips;
            prev_ips = cur_ips;
            cur_ips = temp;
            cur_ips.clear();

            thread::sleep(Duration::from_secs(repeat_interval));
        } else {
            break;
        }
    }

    Ok(())
}
