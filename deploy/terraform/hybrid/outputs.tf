output "server_ipv4" {
  description = "Public IPv4 of Borg host."
  value       = hcloud_server.borg.ipv4_address
}

output "ssh_command" {
  description = "SSH command for host access."
  value       = "ssh root@${hcloud_server.borg.ipv4_address}"
}

output "app_url" {
  description = "External app URL."
  value       = "https://${local.fqdn}"
}

output "cloudflare_tunnel_id" {
  description = "Tunnel UUID when tunnel creation is enabled."
  value       = var.create_tunnel ? cloudflare_zero_trust_tunnel_cloudflared.borg[0].id : null
}

output "cloudflare_tunnel_token" {
  description = "Token used by cloudflared service install."
  value       = var.create_tunnel ? cloudflare_zero_trust_tunnel_cloudflared_token.borg[0].token : null
  sensitive   = true
}

output "r2_bucket_name" {
  description = "R2 bucket name if created."
  value       = var.create_r2_bucket ? cloudflare_r2_bucket.borg[0].name : null
}
