provider "hcloud" {
  token = var.hcloud_token
}

provider "cloudflare" {
  api_token = var.cloudflare_api_token
}

locals {
  fqdn = "${var.app_subdomain}.${var.domain}"
}

resource "hcloud_ssh_key" "borg" {
  name       = "${var.server_name}-key"
  public_key = var.ssh_public_key
}

resource "hcloud_firewall" "borg" {
  name = "${var.server_name}-fw"

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = var.allowed_ssh_cidrs
  }
}

resource "hcloud_server" "borg" {
  name        = var.server_name
  server_type = var.server_type
  image       = var.server_image
  location    = var.server_location
  ssh_keys    = [hcloud_ssh_key.borg.id]
  user_data   = file("${path.module}/../../cloud-init.yml")
}

resource "hcloud_firewall_attachment" "borg" {
  firewall_id = hcloud_firewall.borg.id
  server_ids  = [hcloud_server.borg.id]
}

resource "random_id" "tunnel_secret" {
  count       = var.create_tunnel ? 1 : 0
  byte_length = 32
}

resource "cloudflare_zero_trust_tunnel_cloudflared" "borg" {
  count      = var.create_tunnel ? 1 : 0
  account_id = var.cloudflare_account_id
  name       = "${var.server_name}-tunnel"
  secret     = random_id.tunnel_secret[0].b64_std
}

resource "cloudflare_zero_trust_tunnel_cloudflared_config" "borg" {
  count      = var.create_tunnel ? 1 : 0
  account_id = var.cloudflare_account_id
  tunnel_id  = cloudflare_zero_trust_tunnel_cloudflared.borg[0].id

  config = {
    ingress = [
      {
        hostname = local.fqdn
        service  = "http://localhost:3131"
      },
      {
        service = "http_status:404"
      }
    ]
  }
}

resource "cloudflare_zero_trust_tunnel_cloudflared_token" "borg" {
  count      = var.create_tunnel ? 1 : 0
  account_id = var.cloudflare_account_id
  tunnel_id  = cloudflare_zero_trust_tunnel_cloudflared.borg[0].id
}

resource "cloudflare_dns_record" "app" {
  count   = var.create_tunnel ? 1 : 0
  zone_id = var.cloudflare_zone_id
  name    = var.app_subdomain
  type    = "CNAME"
  content = "${cloudflare_zero_trust_tunnel_cloudflared.borg[0].id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
}

resource "cloudflare_zero_trust_access_application" "app" {
  count      = var.create_tunnel && length(var.cloudflare_access_emails) > 0 ? 1 : 0
  account_id = var.cloudflare_account_id
  name       = "${var.server_name}-app"
  domain     = local.fqdn
  type       = "self_hosted"
}

resource "cloudflare_zero_trust_access_policy" "allow_emails" {
  count          = var.create_tunnel && length(var.cloudflare_access_emails) > 0 ? 1 : 0
  account_id     = var.cloudflare_account_id
  application_id = cloudflare_zero_trust_access_application.app[0].id
  name           = "allow-emails"
  precedence     = 1
  decision       = "allow"

  include = [
    {
      email = {
        email = var.cloudflare_access_emails
      }
    }
  ]
}

resource "cloudflare_r2_bucket" "borg" {
  count      = var.create_r2_bucket ? 1 : 0
  account_id = var.cloudflare_account_id
  name       = var.r2_bucket_name
  location   = "WEUR"
}
