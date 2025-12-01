# cf-dns-rs

Cloudflare Dynamic DNS update tool

Periodically checks the current external IP (v4 and/or v6) and updates the Cloudflare DNS records.

## Build and run

The tool needs an `.env` file present in the root project directory, containing the CloudFlare and Run configurations.

You can run the app as a stand-alone process, in which case the `dotenv` crate will take care of loading the `.env` file into the environment.

```sh
git clone https://github.com/BogdanOlar/cf-dns-rs.git
cd ./cf-dns-rs
cargo run --release
```

You can also build a Docker container, in which case the `.env` file will be automatically loaded.

```sh
git clone https://github.com/BogdanOlar/cf-dns-rs.git
cd ./cf-dns-rs
sudo docker compose up -d --no-deps --build
```

## Example `.env` file

```sh
# IP API endpoints.
# At least one of `IPV4_ENDPOINT` or `IPV6_ENDPOINT` must be
# defined. The 2 endpoints determine which DNS record types will
# be updated (e.g. if only `IPV4_ENDPOINT` is defined, then only
# `A` records will be updated)
IPV4_ENDPOINT=https://api.ipify.org
#IPV6_ENDPOINT=https://api6.ipify.org

# Timeout interval between IP change checks. An interval of `0`
# will cause the app to only run once and then exit. If the
# repeat interval is not defined, then it defaults to `0`
REPEAT_INTERVAL_SECONDS=60

# Cloudflare zone ID (see your account's "Overview" page to get
# the zone ID)
CF_DNS_ZONE_ID=zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz

# Cloudflare API token. Make sure it has DNS Read and Write
# permissions when you create it.
CF_DNS_API_TOKEN=yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy

# List of `;` separated DNS record names which will be updated
CF_DNS_HOSTS=example.com;yyyyyyy.example.com;*.zzzzz.example.com

# By default this app will only update already existing DNS
# records.
# Uncomment the line below to allow the app to create new records,
# if it cannot find one of the hosts above in the existing record
# list
#CF_DNS_CREATE_HOST_RECORDS=true
```
