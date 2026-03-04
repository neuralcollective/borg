variable "hcloud_token" {
  description = "Hetzner Cloud API token."
  type        = string
  sensitive   = true
}

variable "cloudflare_api_token" {
  description = "Cloudflare API token with Zone + Zero Trust permissions."
  type        = string
  sensitive   = true
}

variable "cloudflare_account_id" {
  description = "Cloudflare account ID for Zero Trust resources."
  type        = string
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID for DNS record management."
  type        = string
}

variable "domain" {
  description = "Root domain (example: borg.legal)."
  type        = string
}

variable "app_subdomain" {
  description = "App subdomain label."
  type        = string
  default     = "app"
}

variable "server_name" {
  description = "Hetzner server name."
  type        = string
  default     = "borg"
}

variable "server_type" {
  description = "Hetzner server type."
  type        = string
  default     = "cax21"
}

variable "server_image" {
  description = "Hetzner image."
  type        = string
  default     = "ubuntu-24.04"
}

variable "server_location" {
  description = "Hetzner location."
  type        = string
  default     = "nbg1"
}

variable "ssh_public_key" {
  description = "SSH public key content used for root access."
  type        = string
}

variable "allowed_ssh_cidrs" {
  description = "CIDRs allowed to SSH to the host."
  type        = list(string)
  default     = ["0.0.0.0/0", "::/0"]
}

variable "create_tunnel" {
  description = "Whether to create and configure a Cloudflare tunnel."
  type        = bool
  default     = true
}

variable "cloudflare_access_emails" {
  description = "Emails allowed through Cloudflare Access."
  type        = list(string)
  default     = []
}

variable "create_r2_bucket" {
  description = "Whether to create an R2 bucket for object storage."
  type        = bool
  default     = false
}

variable "r2_bucket_name" {
  description = "R2 bucket name when create_r2_bucket=true."
  type        = string
  default     = ""
}
