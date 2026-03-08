//! Synthetic GCP telemetry emitter.
//!
//! Implements the `parallax_connect::Connector` trait to emit realistic GCP
//! entity and relationship data using the exact type/class/verb contracts
//! defined by the Parallax schema. No real GCP credentials required.
//!
//! ## Entity types emitted
//!
//! | Type                        | Class    | Notes |
//! |-----------------------------|----------|-------|
//! | `gcp_compute_instance`      | `Host`   | Compute Engine VM instances |
//! | `gcp_iam_user`              | `User`   | IAM human principals |
//! | `gcp_iam_service_account`   | `User`   | GCP service accounts |
//! | `gcp_iam_role`              | `Role`   | IAM roles (primitive + predefined) |
//! | `gcp_storage_bucket`        | `DataStore` | Cloud Storage buckets |
//! | `gcp_firewall_rule`         | `Firewall`  | VPC firewall rules |
//! | `edr_agent`                 | `Agent`  | EDR agents covering compute instances |
//!
//! ## Relationships emitted
//!
//! | From                      | Verb       | To                        | Meaning |
//! |---------------------------|------------|---------------------------|---------|
//! | `gcp_iam_user`            | `ASSIGNED` | `gcp_iam_role`            | IAM binding |
//! | `gcp_iam_service_account` | `ASSIGNED` | `gcp_iam_role`            | IAM binding |
//! | `gcp_compute_instance`    | `USES`     | `gcp_iam_service_account` | Runs-as SA |
//! | `gcp_compute_instance`    | `HAS`      | `gcp_firewall_rule`       | Network rule |
//! | `edr_agent`               | `PROTECTS` | `gcp_compute_instance`    | EDR coverage |
//!
//! ## Scenario controls
//!
//! ```rust
//! use connector_gcp_synthetic::SyntheticConfig;
//!
//! let config = SyntheticConfig {
//!     instance_count: 150,
//!     edr_coverage: 0.6,           // 40% of VMs lack EDR (policy violation)
//!     mfa_compliance: 0.5,         // 50% of users lack 2SV (policy violation)
//!     public_bucket_ratio: 0.15,   // 15% of buckets are public (policy violation)
//!     admin_role_ratio: 0.10,      // 10% of SAs have owner/editor binding
//!     ..SyntheticConfig::default()
//! };
//! ```

use async_trait::async_trait;
use parallax_connect::{
    builder::{entity, relationship},
    connector::{step, Connector, StepContext, StepDefinition},
    error::ConnectorError,
};

// ─── Static lookup tables ─────────────────────────────────────────────────────

const MACHINE_TYPES: &[&str] = &[
    "e2-micro", "e2-small", "e2-medium", "e2-standard-2", "e2-standard-4",
    "n1-standard-1", "n1-standard-2", "n1-standard-4", "n1-standard-8",
    "n2-standard-2", "n2-standard-4", "n2-standard-8",
    "c2-standard-4", "c2-standard-8",
    "m1-megamem-96",
];

const REGIONS: &[&str] = &[
    "us-central1",
    "us-east1",
    "us-west1",
    "us-west2",
    "europe-west1",
    "europe-west2",
    "asia-east1",
    "asia-northeast1",
];

// Parallel zone lists, indexed with REGIONS.
const ZONES: &[&[&str]] = &[
    &["us-central1-a", "us-central1-b", "us-central1-c", "us-central1-f"],
    &["us-east1-b", "us-east1-c", "us-east1-d"],
    &["us-west1-a", "us-west1-b", "us-west1-c"],
    &["us-west2-a", "us-west2-b", "us-west2-c"],
    &["europe-west1-b", "europe-west1-c", "europe-west1-d"],
    &["europe-west2-a", "europe-west2-b", "europe-west2-c"],
    &["asia-east1-a", "asia-east1-b", "asia-east1-c"],
    &["asia-northeast1-a", "asia-northeast1-b", "asia-northeast1-c"],
];

const INSTANCE_STATES: &[(&str, u8)] = &[
    ("RUNNING",     75),
    ("TERMINATED",  10),
    ("STOPPED",     10),
    ("STAGING",      5),
];

const STORAGE_CLASSES: &[&str] = &[
    "STANDARD", "NEARLINE", "COLDLINE", "ARCHIVE",
];

const BUCKET_ADJECTIVES: &[&str] = &[
    "prod", "staging", "dev", "archive", "backup",
    "logs", "assets", "data", "shared", "infra",
];

const BUCKET_NOUNS: &[&str] = &[
    "artifacts", "reports", "exports", "uploads",
    "media", "configs", "builds", "audit", "events",
];

const USER_FIRST: &[&str] = &[
    "alice", "bob", "carol", "dave", "eve", "frank",
    "grace", "henry", "irene", "jack", "karen", "liam",
    "mia", "nora", "oscar", "petra", "quinn", "raj",
    "sara", "tom", "uma", "victor", "wendy", "xavier",
];

const USER_DOMAINS: &[&str] = &[
    "example.com", "corp.example.com", "eng.example.com", "admin.example.com",
];

const SA_PREFIXES: &[&str] = &[
    "compute-engine", "cloudbuild", "dataflow", "pubsub",
    "bigquery", "gke-node", "cloud-run", "app-deploy",
    "monitoring", "logging",
];

/// GCP predefined roles that are high-privilege.
const ADMIN_ROLES: &[&str] = &[
    "roles/owner",
    "roles/editor",
    "roles/iam.securityAdmin",
    "roles/iam.serviceAccountAdmin",
    "roles/compute.admin",
    "roles/storage.admin",
];

/// GCP predefined roles that are read-only or scoped.
const LIMITED_ROLES: &[&str] = &[
    "roles/viewer",
    "roles/compute.viewer",
    "roles/storage.objectViewer",
    "roles/storage.objectCreator",
    "roles/logging.viewer",
    "roles/monitoring.viewer",
    "roles/pubsub.subscriber",
    "roles/bigquery.dataViewer",
    "roles/cloudbuild.builds.viewer",
    "roles/run.invoker",
];

const FIREWALL_TARGETS: &[&str] = &[
    "http-server", "https-server", "ssh", "rdp",
    "internal", "lb-health-check", "allow-ssh-iap",
    "allow-internal", "deny-all-ingress",
];

const EDR_VENDORS: &[&str] = &[
    "CrowdStrike Falcon",
    "SentinelOne",
    "Carbon Black",
    "Microsoft Defender",
];

// ─── LCG PRNG (no external dependency) ───────────────────────────────────────

/// Minimal seeded pseudo-random number generator (Knuth multiplicative LCG).
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        let s = seed.wrapping_add(0x9e3779b97f4a7c15);
        let mut l = Lcg(s);
        l.next(); l.next();
        l
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next() as usize % items.len()]
    }

    fn prob(&mut self, p: f64) -> bool {
        (self.next() as f64 / u64::MAX as f64) < p
    }

    fn digits(&mut self, n: usize) -> String {
        let mut s = String::with_capacity(n);
        while s.len() < n {
            s.push_str(&format!("{:020}", self.next()));
        }
        s.truncate(n);
        s
    }

    fn pick_weighted<'a, T>(&mut self, table: &'a [(&'a T, u8)]) -> &'a T
    where
        T: ?Sized,
    {
        let total: u32 = table.iter().map(|(_, w)| *w as u32).sum();
        let mut roll = self.next() as u32 % total;
        for (val, w) in table {
            if roll < *w as u32 {
                return val;
            }
            roll -= *w as u32;
        }
        table.last().expect("non-empty table").0
    }
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for the synthetic GCP emitter.
///
/// All ratio fields are in range \[0.0, 1.0\].
/// The seed controls all randomness — same seed always produces the same graph.
#[derive(Debug, Clone)]
pub struct SyntheticConfig {
    /// RNG seed. Default: `42`.
    pub seed: u64,

    /// GCP project ID.
    pub project_id: String,

    /// GCP project number (numeric).
    pub project_number: String,

    /// Primary region.
    pub region: String,

    // ── Scale ──────────────────────────────────────────────────────────────────
    pub instance_count: usize,
    pub user_count: usize,
    pub service_account_count: usize,
    pub bucket_count: usize,
    /// Number of firewall rules. Applied to instances via network tags.
    pub firewall_rule_count: usize,

    // ── Scenario controls ─────────────────────────────────────────────────────
    /// Fraction of Compute instances covered by an EDR agent.
    pub edr_coverage: f64,

    /// Fraction of IAM users with 2-Step Verification active.
    pub mfa_compliance: f64,

    /// Fraction of Cloud Storage buckets with `allUsers` public access.
    pub public_bucket_ratio: f64,

    /// Fraction of service accounts with admin/owner/editor IAM binding.
    pub admin_role_ratio: f64,

    /// Fraction of Compute instances with OS Login enabled.
    pub os_login_ratio: f64,
}

impl Default for SyntheticConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            project_id: "my-project-123456".to_owned(),
            project_number: "123456789012".to_owned(),
            region: "us-central1".to_owned(),
            instance_count: 80,
            user_count: 20,
            service_account_count: 15,
            bucket_count: 18,
            firewall_rule_count: 10,
            edr_coverage: 0.78,
            mfa_compliance: 0.72,
            public_bucket_ratio: 0.08,
            admin_role_ratio: 0.15,
            os_login_ratio: 0.65,
        }
    }
}

// ─── Pre-computed entity records ──────────────────────────────────────────────

struct InstanceRecord {
    name: String,
    instance_id: String,      // numeric instance ID
    machine_type: &'static str,
    status: &'static str,
    region_idx: usize,
    zone: &'static str,
    private_ip: String,
    public_ip: Option<String>,
    has_edr: bool,
    os_login: bool,
    sa_idx: Option<usize>,    // service account binding
    fw_idx: usize,            // primary firewall rule
}

struct UserRecord {
    email: String,
    display_name: String,
    mfa_active: bool,
    active: bool,
    role_indices: Vec<usize>,
}

struct ServiceAccountRecord {
    email: String,
    display_name: String,
    disabled: bool,
    key_count: u32,
    role_indices: Vec<usize>,
}

struct RoleRecord {
    role_id: String,     // e.g. "roles/compute.admin" or "projects/p/roles/custom-001"
    title: String,
    is_admin: bool,
    is_custom: bool,
}

struct BucketRecord {
    name: String,
    location: String,
    storage_class: &'static str,
    public: bool,
    versioning: bool,
    logging: bool,
    uniform_acl: bool,
}

struct FirewallRecord {
    name: String,
    rule_id: String,
    direction: &'static str,  // "INGRESS" or "EGRESS"
    action: &'static str,     // "ALLOW" or "DENY"
    priority: u32,
    target_tag: &'static str,
}

// ─── Connector ────────────────────────────────────────────────────────────────

/// Synthetic GCP connector.
///
/// All entity data is pre-computed from the seeded RNG in `new()`. Each
/// `execute_step` call reads from the pre-computed records — fast and
/// deterministic. The connector is `Send + Sync` and can be wrapped in `Arc`.
pub struct GcpSyntheticConnector {
    config: SyntheticConfig,
    instances: Vec<InstanceRecord>,
    users: Vec<UserRecord>,
    service_accounts: Vec<ServiceAccountRecord>,
    roles: Vec<RoleRecord>,
    buckets: Vec<BucketRecord>,
    firewall_rules: Vec<FirewallRecord>,
}

impl GcpSyntheticConnector {
    /// Build the connector, pre-generating all entity data from `config.seed`.
    pub fn new(config: SyntheticConfig) -> Self {
        let mut rng = Lcg::new(config.seed);

        // IAM roles — split between admin and limited.
        let total_roles = (config.service_account_count + config.user_count).max(6);
        let n_admin = ((total_roles as f64 * 0.30) as usize).max(2);
        let n_limited = total_roles - n_admin;

        let mut roles: Vec<RoleRecord> = Vec::with_capacity(total_roles);
        for i in 0..n_admin {
            let role_id = (*rng.pick(ADMIN_ROLES)).to_owned();
            let title = role_id.trim_start_matches("roles/").replace('.', " - ");
            roles.push(RoleRecord {
                role_id,
                title,
                is_admin: true,
                is_custom: false,
            });
            let _ = i;
        }
        for i in 0..n_limited {
            let is_custom = rng.prob(0.20);
            let (role_id, title) = if is_custom {
                let id = format!("projects/{}/roles/custom-{i:03}", config.project_id);
                let title = format!("Custom Role {i:03}");
                (id, title)
            } else {
                let r = (*rng.pick(LIMITED_ROLES)).to_owned();
                let title = r.trim_start_matches("roles/").replace('.', " - ");
                (r, title)
            };
            roles.push(RoleRecord {
                role_id,
                title,
                is_admin: false,
                is_custom,
            });
        }

        // Service accounts.
        let service_accounts: Vec<ServiceAccountRecord> = (0..config.service_account_count)
            .map(|i| {
                let prefix = rng.pick(SA_PREFIXES);
                let email = format!(
                    "{prefix}-sa-{i:03}@{}.iam.gserviceaccount.com",
                    config.project_id
                );
                let display_name = format!("{prefix} Service Account {i:03}");
                let disabled = rng.prob(0.05);
                let key_count = (rng.next() % 4) as u32;
                // Assign 1–2 roles, biased toward admin_role_ratio.
                let n_roles = (rng.next() as usize % 2) + 1;
                let role_indices: Vec<usize> = (0..n_roles)
                    .map(|_| {
                        if rng.prob(config.admin_role_ratio) {
                            // Pick from admin roles (first n_admin).
                            rng.next() as usize % n_admin
                        } else {
                            n_admin + rng.next() as usize % n_limited
                        }
                    })
                    .collect();
                ServiceAccountRecord { email, display_name, disabled, key_count, role_indices }
            })
            .collect();

        // IAM users.
        let users: Vec<UserRecord> = (0..config.user_count)
            .map(|i| {
                let first = rng.pick(USER_FIRST);
                let domain = rng.pick(USER_DOMAINS);
                let email = if config.user_count <= USER_FIRST.len() {
                    format!("{first}@{domain}")
                } else {
                    format!("{first}{i:02}@{domain}")
                };
                let display_name = {
                    let mut s = first.to_string();
                    s[..1].make_ascii_uppercase();
                    s
                };
                let mfa_active = rng.prob(config.mfa_compliance);
                let active = rng.prob(0.92);
                let n_roles = (rng.next() as usize % 3) + 1;
                let role_indices: Vec<usize> = (0..n_roles)
                    .map(|_| rng.next() as usize % roles.len())
                    .collect();
                UserRecord { email, display_name, mfa_active, active, role_indices }
            })
            .collect();

        // Cloud Storage buckets.
        let buckets: Vec<BucketRecord> = (0..config.bucket_count)
            .map(|i| {
                let adj = rng.pick(BUCKET_ADJECTIVES);
                let noun = rng.pick(BUCKET_NOUNS);
                let proj = &config.project_id;
                let loc_region = rng.pick(REGIONS);
                BucketRecord {
                    name: format!("{proj}-{adj}-{noun}-{i:03}"),
                    location: loc_region.to_uppercase(),
                    storage_class: *rng.pick(STORAGE_CLASSES),
                    public: rng.prob(config.public_bucket_ratio),
                    versioning: rng.prob(0.55),
                    logging: rng.prob(0.45),
                    uniform_acl: rng.prob(0.70),
                }
            })
            .collect();

        // Firewall rules.
        let firewall_rules: Vec<FirewallRecord> = (0..config.firewall_rule_count)
            .map(|i| {
                let direction = if rng.prob(0.75) { "INGRESS" } else { "EGRESS" };
                let action = if rng.prob(0.85) { "ALLOW" } else { "DENY" };
                let priority = 1000u32 + (rng.next() % 64000) as u32;
                let target_tag = *rng.pick(FIREWALL_TARGETS);
                FirewallRecord {
                    name: format!("fw-rule-{i:03}-{}", target_tag.replace('-', "")),
                    rule_id: rng.digits(19),
                    direction,
                    action,
                    priority,
                    target_tag,
                }
            })
            .collect();

        // Compute instances.
        let state_table: Vec<(&str, u8)> = INSTANCE_STATES.iter().map(|(s, w)| (*s, *w)).collect();

        let instances: Vec<InstanceRecord> = (0..config.instance_count)
            .map(|i| {
                let region_idx = rng.next() as usize % REGIONS.len();
                let zone = *rng.pick(ZONES[region_idx]);
                let machine_type = rng.pick(MACHINE_TYPES);
                let status = *rng.pick_weighted(
                    &state_table.iter().map(|(s, w)| (s, *w)).collect::<Vec<_>>(),
                );
                let has_public = rng.prob(0.30);
                let private_ip = format!(
                    "10.{}.{}.{}",
                    rng.next() % 4,
                    rng.next() % 256,
                    rng.next() % 256
                );
                let public_ip = if has_public {
                    Some(format!(
                        "{}.{}.{}.{}",
                        34 + rng.next() % 180,
                        rng.next() % 256,
                        rng.next() % 256,
                        rng.next() % 256
                    ))
                } else {
                    None
                };
                let sa_idx = if rng.prob(0.85) {
                    Some(rng.next() as usize % service_accounts.len())
                } else {
                    None
                };
                let fw_idx = rng.next() as usize % firewall_rules.len();
                InstanceRecord {
                    name: format!("instance-{i:04}"),
                    instance_id: rng.digits(19),
                    machine_type,
                    status,
                    region_idx,
                    zone,
                    private_ip,
                    public_ip,
                    has_edr: rng.prob(config.edr_coverage),
                    os_login: rng.prob(config.os_login_ratio),
                    sa_idx,
                    fw_idx,
                }
            })
            .collect();

        GcpSyntheticConnector {
            config,
            instances,
            users,
            service_accounts,
            roles,
            buckets,
            firewall_rules,
        }
    }
}

#[async_trait]
impl Connector for GcpSyntheticConnector {
    fn name(&self) -> &str {
        "gcp-synthetic"
    }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            // Wave 0 — all independent, run concurrently.
            step("compute-instances",  "Emit Compute Engine instances").build(),
            step("iam-users",          "Emit IAM user principals").build(),
            step("service-accounts",   "Emit IAM service accounts").build(),
            step("iam-roles",          "Emit IAM roles").build(),
            step("storage-buckets",    "Emit Cloud Storage buckets").build(),
            step("firewall-rules",     "Emit VPC firewall rules").build(),
            // Wave 1 — relationships + EDR agents reference wave-0 entity keys.
            step("edr-agents", "Emit EDR agents and PROTECTS relationships")
                .depends_on(&["compute-instances"])
                .build(),
            step("iam-relationships", "Emit IAM binding relationships")
                .depends_on(&["iam-users", "service-accounts", "iam-roles"])
                .build(),
            step("compute-relationships", "Emit compute SA and firewall relationships")
                .depends_on(&["compute-instances", "service-accounts", "firewall-rules"])
                .build(),
        ]
    }

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        match step_id {
            "compute-instances"    => self.emit_compute_instances(ctx),
            "iam-users"            => self.emit_iam_users(ctx),
            "service-accounts"     => self.emit_service_accounts(ctx),
            "iam-roles"            => self.emit_iam_roles(ctx),
            "storage-buckets"      => self.emit_storage_buckets(ctx),
            "firewall-rules"       => self.emit_firewall_rules(ctx),
            "edr-agents"           => self.emit_edr_agents(ctx),
            "iam-relationships"    => self.emit_iam_relationships(ctx),
            "compute-relationships" => self.emit_compute_relationships(ctx),
            other => Err(ConnectorError::UnknownStep(other.to_owned())),
        }
    }
}

// ─── Step implementations ─────────────────────────────────────────────────────

impl GcpSyntheticConnector {
    fn emit_compute_instances(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for r in &self.instances {
            let region = REGIONS[r.region_idx];
            let self_link = format!(
                "https://www.googleapis.com/compute/v1/projects/{}/zones/{}/instances/{}",
                self.config.project_id, r.zone, r.name
            );
            let mut e = entity("gcp_compute_instance", &r.name)
                .class("Host")
                .display_name(&r.name)
                .property("instanceId", r.instance_id.as_str())
                .property("machineType", r.machine_type)
                .property("status", r.status)
                .property("zone", r.zone)
                .property("region", region)
                .property("projectId", self.config.project_id.as_str())
                .property("privateIpAddress", r.private_ip.as_str())
                .property("osLogin", r.os_login)
                .property("selfLink", self_link.as_str())
                .property("active", r.status == "RUNNING");

            if let Some(ip) = &r.public_ip {
                e = e.property("publicIpAddress", ip.as_str());
            }

            ctx.emit_entity(e)?;
        }
        Ok(())
    }

    fn emit_iam_users(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for u in &self.users {
            ctx.emit_entity(
                entity("gcp_iam_user", &u.email)
                    .class("User")
                    .display_name(&u.display_name)
                    .property("email", u.email.as_str())
                    .property("projectId", self.config.project_id.as_str())
                    .property("mfaActive", u.mfa_active)
                    .property("active", u.active),
            )?;
        }
        Ok(())
    }

    fn emit_service_accounts(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for sa in &self.service_accounts {
            let self_link = format!(
                "https://iam.googleapis.com/v1/projects/{}/serviceAccounts/{}",
                self.config.project_id, sa.email
            );
            ctx.emit_entity(
                entity("gcp_iam_service_account", &sa.email)
                    .class("User")
                    .display_name(&sa.display_name)
                    .property("email", sa.email.as_str())
                    .property("projectId", self.config.project_id.as_str())
                    .property("disabled", sa.disabled)
                    .property("keyCount", sa.key_count as i64)
                    .property("selfLink", self_link.as_str())
                    .property("active", !sa.disabled),
            )?;
        }
        Ok(())
    }

    fn emit_iam_roles(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for r in &self.roles {
            ctx.emit_entity(
                entity("gcp_iam_role", &r.role_id)
                    .class("Role")
                    .display_name(&r.title)
                    .property("roleId", r.role_id.as_str())
                    .property("title", r.title.as_str())
                    .property("admin", r.is_admin)
                    .property("isCustom", r.is_custom)
                    .property("projectId", self.config.project_id.as_str())
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_storage_buckets(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for b in &self.buckets {
            let self_link = format!(
                "https://www.googleapis.com/storage/v1/b/{}",
                b.name
            );
            ctx.emit_entity(
                entity("gcp_storage_bucket", &b.name)
                    .class("DataStore")
                    .display_name(&b.name)
                    .property("location", b.location.as_str())
                    .property("storageClass", b.storage_class)
                    .property("public", b.public)
                    .property("versioning", b.versioning)
                    .property("logging", b.logging)
                    .property("uniformBucketLevelAccess", b.uniform_acl)
                    .property("projectId", self.config.project_id.as_str())
                    .property("selfLink", self_link.as_str())
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_firewall_rules(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for fw in &self.firewall_rules {
            let self_link = format!(
                "https://www.googleapis.com/compute/v1/projects/{}/global/firewalls/{}",
                self.config.project_id, fw.name
            );
            ctx.emit_entity(
                entity("gcp_firewall_rule", &fw.name)
                    .class("Firewall")
                    .display_name(&fw.name)
                    .property("ruleId", fw.rule_id.as_str())
                    .property("direction", fw.direction)
                    .property("action", fw.action)
                    .property("priority", fw.priority as i64)
                    .property("targetTag", fw.target_tag)
                    .property("projectId", self.config.project_id.as_str())
                    .property("selfLink", self_link.as_str())
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_edr_agents(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        let mut rng = Lcg::new(self.config.seed.wrapping_add(0xCAFEBABE));

        for (i, inst) in self.instances.iter().enumerate() {
            if !inst.has_edr {
                continue;
            }
            let vendor = rng.pick(EDR_VENDORS);
            let agent_id = format!("gcp-agent-{i:05}");
            let version = format!(
                "{}.{}.{}",
                rng.next() % 8,
                rng.next() % 20,
                rng.next() % 100
            );

            ctx.emit_entity(
                entity("edr_agent", &agent_id)
                    .class("Agent")
                    .display_name(&format!("{vendor} on {}", inst.name))
                    .property("vendor", vendor.as_ref())
                    .property("version", version.as_str())
                    .property("active", true),
            )?;

            ctx.emit_relationship(
                relationship(&agent_id, "PROTECTS", &inst.name)
                    .from_type("edr_agent")
                    .to_type("gcp_compute_instance"),
            )?;
        }
        Ok(())
    }

    fn emit_iam_relationships(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        // gcp_iam_user ASSIGNED gcp_iam_role
        for user in &self.users {
            for &role_idx in &user.role_indices {
                let role = &self.roles[role_idx];
                ctx.emit_relationship(
                    relationship(&user.email, "ASSIGNED", &role.role_id)
                        .from_type("gcp_iam_user")
                        .to_type("gcp_iam_role"),
                )?;
            }
        }

        // gcp_iam_service_account ASSIGNED gcp_iam_role
        for sa in &self.service_accounts {
            for &role_idx in &sa.role_indices {
                let role = &self.roles[role_idx];
                ctx.emit_relationship(
                    relationship(&sa.email, "ASSIGNED", &role.role_id)
                        .from_type("gcp_iam_service_account")
                        .to_type("gcp_iam_role"),
                )?;
            }
        }
        Ok(())
    }

    fn emit_compute_relationships(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for inst in &self.instances {
            // gcp_compute_instance USES gcp_iam_service_account
            if let Some(sa_idx) = inst.sa_idx {
                let sa = &self.service_accounts[sa_idx];
                ctx.emit_relationship(
                    relationship(&inst.name, "USES", &sa.email)
                        .from_type("gcp_compute_instance")
                        .to_type("gcp_iam_service_account"),
                )?;
            }

            // gcp_compute_instance HAS gcp_firewall_rule
            let fw = &self.firewall_rules[inst.fw_idx];
            ctx.emit_relationship(
                relationship(&inst.name, "HAS", &fw.name)
                    .from_type("gcp_compute_instance")
                    .to_type("gcp_firewall_rule"),
            )?;
        }
        Ok(())
    }
}

// ─── Convenience constructors ─────────────────────────────────────────────────

impl GcpSyntheticConnector {
    /// Pre-built "clean" scenario — all policies pass.
    pub fn clean(scale: usize) -> Self {
        Self::new(SyntheticConfig {
            instance_count: scale,
            user_count: scale / 4,
            service_account_count: scale / 5,
            bucket_count: scale / 4,
            edr_coverage: 1.0,
            mfa_compliance: 1.0,
            public_bucket_ratio: 0.0,
            admin_role_ratio: 0.0,
            ..SyntheticConfig::default()
        })
    }

    /// Pre-built "worst case" scenario — maximum policy violations.
    pub fn worst_case(scale: usize) -> Self {
        Self::new(SyntheticConfig {
            instance_count: scale,
            user_count: scale / 4,
            service_account_count: scale / 5,
            bucket_count: scale / 4,
            edr_coverage: 0.0,
            mfa_compliance: 0.0,
            public_bucket_ratio: 1.0,
            admin_role_ratio: 1.0,
            ..SyntheticConfig::default()
        })
    }

    /// Pre-built scenario matching realistic enterprise GCP posture.
    pub fn realistic(scale: usize) -> Self {
        Self::new(SyntheticConfig {
            instance_count: scale,
            user_count: scale / 3,
            service_account_count: scale / 4,
            bucket_count: scale / 3,
            edr_coverage: 0.80,
            mfa_compliance: 0.78,
            public_bucket_ratio: 0.06,
            admin_role_ratio: 0.10,
            ..SyntheticConfig::default()
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_connector() -> GcpSyntheticConnector {
        GcpSyntheticConnector::new(SyntheticConfig::default())
    }

    #[test]
    fn entity_counts_match_config() {
        let cfg = SyntheticConfig {
            instance_count: 10,
            user_count: 5,
            service_account_count: 4,
            bucket_count: 6,
            firewall_rule_count: 3,
            ..SyntheticConfig::default()
        };
        let c = GcpSyntheticConnector::new(cfg.clone());
        assert_eq!(c.instances.len(), cfg.instance_count);
        assert_eq!(c.users.len(), cfg.user_count);
        assert_eq!(c.service_accounts.len(), cfg.service_account_count);
        assert_eq!(c.buckets.len(), cfg.bucket_count);
        assert_eq!(c.firewall_rules.len(), cfg.firewall_rule_count);
    }

    #[test]
    fn deterministic_same_seed() {
        let a = GcpSyntheticConnector::new(SyntheticConfig { seed: 7, ..SyntheticConfig::default() });
        let b = GcpSyntheticConnector::new(SyntheticConfig { seed: 7, ..SyntheticConfig::default() });
        assert_eq!(a.instances[0].name, b.instances[0].name);
        assert_eq!(a.instances[0].instance_id, b.instances[0].instance_id);
        assert_eq!(a.users[0].email, b.users[0].email);
    }

    #[test]
    fn different_seeds_produce_different_data() {
        let a = GcpSyntheticConnector::new(SyntheticConfig { seed: 1, ..SyntheticConfig::default() });
        let b = GcpSyntheticConnector::new(SyntheticConfig { seed: 2, ..SyntheticConfig::default() });
        assert_ne!(a.instances[0].instance_id, b.instances[0].instance_id);
    }

    #[test]
    fn edr_coverage_respected() {
        let cfg = SyntheticConfig {
            instance_count: 200,
            edr_coverage: 0.5,
            seed: 77,
            ..SyntheticConfig::default()
        };
        let c = GcpSyntheticConnector::new(cfg);
        let covered = c.instances.iter().filter(|e| e.has_edr).count();
        assert!(covered >= 80 && covered <= 120, "edr covered={covered}, expected ~100");
    }

    #[test]
    fn clean_scenario_all_covered() {
        let c = GcpSyntheticConnector::clean(20);
        assert!(c.instances.iter().all(|i| i.has_edr));
        assert!(c.users.iter().all(|u| u.mfa_active));
        assert!(c.buckets.iter().all(|b| !b.public));
        assert!(c.roles.iter().filter(|r| r.is_admin).count() == 0
            || c.service_accounts.iter().all(|sa| {
                sa.role_indices.iter().all(|&ri| !c.roles[ri].is_admin)
            }));
    }

    #[test]
    fn worst_case_scenario_all_violated() {
        let c = GcpSyntheticConnector::worst_case(20);
        assert!(c.instances.iter().all(|i| !i.has_edr));
        assert!(c.users.iter().all(|u| !u.mfa_active));
        assert!(c.buckets.iter().all(|b| b.public));
    }

    #[test]
    fn steps_form_valid_wave_structure() {
        use parallax_connect::connector::topological_waves;
        let c = default_connector();
        let waves = topological_waves(&c.steps());
        assert_eq!(waves[0].len(), 6, "wave 0 must have 6 parallel steps");
        assert_eq!(waves[1].len(), 3, "wave 1 must have 3 relationship steps");
    }

    #[tokio::test]
    async fn run_emits_expected_entity_count() {
        use std::sync::Arc;
        use parallax_connect::scheduler::run_connector;

        let cfg = SyntheticConfig {
            instance_count: 10,
            user_count: 4,
            service_account_count: 3,
            bucket_count: 5,
            firewall_rule_count: 2,
            edr_coverage: 0.5,
            ..SyntheticConfig::default()
        };
        let c = Arc::new(GcpSyntheticConnector::new(cfg.clone()));
        let out = run_connector(c, "test-proj", "sync-1", None).await.unwrap();

        let min_entities = cfg.instance_count
            + cfg.user_count
            + cfg.service_account_count
            + cfg.bucket_count
            + cfg.firewall_rule_count;
        assert!(
            out.entities.len() >= min_entities,
            "entities={}, expected ≥{}", out.entities.len(), min_entities
        );
        assert!(!out.relationships.is_empty(), "must emit at least some relationships");
    }

    #[tokio::test]
    async fn entity_types_all_present() {
        use std::sync::Arc;
        use parallax_connect::scheduler::run_connector;
        use std::collections::HashSet;

        let c = Arc::new(GcpSyntheticConnector::new(SyntheticConfig {
            instance_count: 5,
            user_count: 3,
            service_account_count: 3,
            bucket_count: 3,
            firewall_rule_count: 2,
            edr_coverage: 1.0,
            ..SyntheticConfig::default()
        }));
        let out = run_connector(c, "proj", "s1", None).await.unwrap();

        let types: HashSet<&str> = out.entities.iter()
            .map(|e| e._type.as_str())
            .collect();

        for expected in &[
            "gcp_compute_instance",
            "gcp_iam_user",
            "gcp_iam_service_account",
            "gcp_iam_role",
            "gcp_storage_bucket",
            "gcp_firewall_rule",
            "edr_agent",
        ] {
            assert!(types.contains(expected), "missing entity type: {expected}");
        }
    }
}
