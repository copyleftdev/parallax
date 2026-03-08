//! Synthetic AWS telemetry emitter.
//!
//! Implements the `parallax_connect::Connector` trait to emit realistic AWS
//! entity and relationship data using the exact type/class/verb contracts
//! defined by the Parallax schema. No real AWS credentials required.
//!
//! ## Entity types emitted
//!
//! | Type                  | Class       | Notes |
//! |-----------------------|-------------|-------|
//! | `aws_ec2_instance`    | `Host`      | EC2 compute instances |
//! | `aws_iam_user`        | `User`      | IAM human accounts |
//! | `aws_iam_role`        | `Role`      | IAM roles (instance profiles, service roles) |
//! | `aws_iam_policy`      | `AccessPolicy` | Managed IAM policies |
//! | `aws_s3_bucket`       | `DataStore` | S3 storage buckets |
//! | `aws_security_group`  | `Firewall`  | VPC security groups |
//! | `edr_agent`           | `Agent`     | EDR agents covering hosts |
//!
//! ## Relationships emitted
//!
//! | From                | Verb        | To                   | Meaning |
//! |---------------------|-------------|----------------------|---------|
//! | `aws_iam_user`      | `ASSIGNED`  | `aws_iam_role`       | Role assignment |
//! | `aws_iam_role`      | `ASSIGNED`  | `aws_iam_policy`     | Policy attachment |
//! | `aws_ec2_instance`  | `USES`      | `aws_iam_role`       | Instance profile |
//! | `aws_ec2_instance`  | `HAS`       | `aws_security_group` | Security group attachment |
//! | `edr_agent`         | `PROTECTS`  | `aws_ec2_instance`   | EDR coverage |
//!
//! ## Scenario controls
//!
//! ```rust
//! use connector_aws_synthetic::SyntheticConfig;
//!
//! let config = SyntheticConfig {
//!     ec2_count: 200,
//!     edr_coverage: 0.6,       // 40% of hosts lack EDR (policy violation)
//!     mfa_compliance: 0.5,     // 50% of users lack MFA (policy violation)
//!     public_bucket_ratio: 0.2,// 20% of buckets are public (policy violation)
//!     admin_role_ratio: 0.15,  // 15% of roles are admin (policy violation)
//!     ..SyntheticConfig::default()
//! };
//! ```

#![allow(clippy::explicit_auto_deref)]

use async_trait::async_trait;
use parallax_connect::{
    builder::{entity, relationship},
    connector::{step, Connector, StepContext, StepDefinition},
    error::ConnectorError,
};

// ─── Static lookup tables ─────────────────────────────────────────────────────

const INSTANCE_TYPES: &[&str] = &[
    "t3.micro",
    "t3.small",
    "t3.medium",
    "t3.large",
    "t3.xlarge",
    "m5.large",
    "m5.xlarge",
    "m5.2xlarge",
    "m5.4xlarge",
    "c5.large",
    "c5.xlarge",
    "c5.2xlarge",
    "r5.large",
    "r5.xlarge",
    "m6i.large",
    "m6i.xlarge",
];

const REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-2",
    "eu-west-1",
    "eu-central-1",
    "ap-southeast-1",
];

// Parallel AZ lists, indexed with REGIONS.
const AZS: &[&[&str]] = &[
    &["us-east-1a", "us-east-1b", "us-east-1c"],
    &["us-east-2a", "us-east-2b", "us-east-2c"],
    &["us-west-2a", "us-west-2b", "us-west-2c"],
    &["eu-west-1a", "eu-west-1b"],
    &["eu-central-1a", "eu-central-1b"],
    &["ap-southeast-1a", "ap-southeast-1b"],
];

const EC2_STATES: &[(&str, u8)] = &[
    ("running", 75), // weighted: 75% running
    ("stopped", 15),
    ("pending", 5),
    ("terminated", 5),
];

const PLATFORMS: &[(&str, u8)] = &[("linux", 80), ("windows", 20)];

const EDR_VENDORS: &[&str] = &[
    "CrowdStrike Falcon",
    "SentinelOne",
    "Carbon Black",
    "Microsoft Defender",
];

const SG_DESCRIPTIONS: &[&str] = &[
    "Web tier inbound",
    "App tier inbound",
    "DB tier inbound",
    "Management access",
    "Default VPC SG",
    "Bastion host SG",
    "Load balancer SG",
    "Internal microservices",
];

const AWS_MANAGED_POLICIES: &[&str] = &[
    "AdministratorAccess",
    "PowerUserAccess",
    "ReadOnlyAccess",
    "AmazonS3FullAccess",
    "AmazonS3ReadOnlyAccess",
    "AmazonEC2FullAccess",
    "AmazonEC2ReadOnlyAccess",
    "AWSLambdaFullAccess",
    "IAMFullAccess",
    "AmazonRDSFullAccess",
    "AWSSecurityHubFullAccess",
    "AmazonVPCFullAccess",
    "CloudWatchFullAccess",
    "AWSCloudTrailFullAccess",
];

const BUCKET_ADJECTIVES: &[&str] = &[
    "prod", "staging", "dev", "archive", "backup", "logs", "assets", "data", "shared", "infra",
];

const BUCKET_NOUNS: &[&str] = &[
    "artifacts",
    "reports",
    "exports",
    "uploads",
    "media",
    "configs",
    "secrets",
    "audit",
    "events",
];

const USER_FIRST: &[&str] = &[
    "alice", "bob", "carol", "dave", "eve", "frank", "grace", "henry", "irene", "jack", "karen",
    "liam", "mia", "nora", "oscar", "petra", "quinn", "raj", "sara", "tom", "uma", "victor",
    "wendy", "xavier",
];

const ROLE_PREFIXES: &[&str] = &[
    "ec2", "lambda", "ecs", "rds", "s3", "api", "worker", "deploy", "build", "monitor",
];

const ROLE_SUFFIXES: &[&str] = &[
    "readonly",
    "fullaccess",
    "admin",
    "executor",
    "power",
    "limited",
    "service",
];

// ─── LCG PRNG (no external dependency) ───────────────────────────────────────

/// Minimal seeded pseudo-random number generator (Knuth multiplicative LCG).
/// Deterministic and reproducible — same seed always produces the same sequence.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        // Mix the seed to avoid poor initial state with seed = 0.
        let s = seed.wrapping_add(0x9e3779b97f4a7c15);
        let mut l = Lcg(s);
        l.next();
        l.next(); // warm up
        l
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// Pick a random element from a slice.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next() as usize % items.len()]
    }

    /// Return true with probability `p` (0.0–1.0).
    fn prob(&mut self, p: f64) -> bool {
        (self.next() as f64 / u64::MAX as f64) < p
    }

    /// Generate a hex string of length `n`.
    fn hex(&mut self, n: usize) -> String {
        let mut s = String::with_capacity(n + 2);
        while s.len() < n {
            s.push_str(&format!("{:016x}", self.next()));
        }
        s.truncate(n);
        s
    }

    /// Pick from a weighted table of (value, weight) pairs.
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

/// Configuration for the synthetic AWS emitter.
///
/// All ratio fields are in range \[0.0, 1.0\].
/// The seed controls all randomness — same seed always produces the same graph.
#[derive(Debug, Clone)]
pub struct SyntheticConfig {
    /// RNG seed. Default: `42`. Change to get a different but equally valid graph.
    pub seed: u64,

    /// Synthetic AWS account ID (12 digits).
    pub account_id: String,

    /// Primary AWS region (others are generated randomly per resource).
    pub region: String,

    // ── Scale ──────────────────────────────────────────────────────────────────
    pub ec2_count: usize,
    pub iam_user_count: usize,
    pub iam_role_count: usize,
    pub s3_bucket_count: usize,
    /// Number of security groups. Shared across EC2 instances (many-to-one).
    pub sg_count: usize,

    // ── Scenario controls ─────────────────────────────────────────────────────
    /// Fraction of EC2 instances covered by an EDR agent.
    /// `1.0` = full coverage. `0.7` = 30% gap → policy violations.
    pub edr_coverage: f64,

    /// Fraction of IAM users with MFA active.
    /// `1.0` = all compliant. `0.6` = 40% violation.
    pub mfa_compliance: f64,

    /// Fraction of S3 buckets with public access enabled.
    /// `0.0` = all private. `0.1` = 10% public → policy violations.
    pub public_bucket_ratio: f64,

    /// Fraction of IAM roles with `admin = true`.
    /// High values simulate over-privileged role sprawl.
    pub admin_role_ratio: f64,

    /// Fraction of EC2 instances with monitoring enabled (CloudWatch detailed).
    pub monitoring_ratio: f64,
}

impl Default for SyntheticConfig {
    /// Balanced defaults: realistic scale with some policy violations in every
    /// category so policy rules have something to find.
    fn default() -> Self {
        Self {
            seed: 42,
            account_id: "123456789012".to_owned(),
            region: "us-east-1".to_owned(),
            ec2_count: 80,
            iam_user_count: 25,
            iam_role_count: 12,
            s3_bucket_count: 20,
            sg_count: 8,
            edr_coverage: 0.78,
            mfa_compliance: 0.72,
            public_bucket_ratio: 0.10,
            admin_role_ratio: 0.17,
            monitoring_ratio: 0.60,
        }
    }
}

// ─── Pre-computed entity records ──────────────────────────────────────────────

struct Ec2Record {
    instance_id: String, // "i-0abc1234567890abc"
    instance_type: &'static str,
    state: &'static str,
    region_idx: usize,
    az: &'static str,
    platform: &'static str,
    private_ip: String,
    public_ip: Option<String>,
    monitoring: bool,
    vpc_id: String,
    subnet_id: String,
    has_edr: bool,
    role_idx: Option<usize>, // instance profile IAM role
    sg_idx: usize,
}

struct UserRecord {
    username: String,
    user_id: String, // "AIDA..."
    mfa_active: bool,
    active: bool,
    password_last_used_days: u32,
    role_indices: Vec<usize>, // assigned roles
}

struct RoleRecord {
    role_name: String,
    role_id: String, // "AROA..."
    is_admin: bool,
    policy_indices: Vec<usize>, // attached policies
}

struct PolicyRecord {
    policy_name: &'static str,
    policy_id: String, // "ANPA..."
    is_aws_managed: bool,
}

struct BucketRecord {
    name: String,
    region_idx: usize,
    public: bool,
    versioning: bool,
    logging: bool,
    encryption: bool,
}

struct SgRecord {
    sg_id: String,
    name: String,
    description: &'static str,
    vpc_id: String,
    allows_all_outbound: bool,
}

// ─── Connector ────────────────────────────────────────────────────────────────

/// Synthetic AWS connector.
///
/// All entity data is pre-computed from the seeded RNG in `new()`. Each
/// `execute_step` call reads from the pre-computed records — fast and
/// deterministic. The connector is `Send + Sync` and can be wrapped in `Arc`.
pub struct AwsSyntheticConnector {
    config: SyntheticConfig,
    ec2: Vec<Ec2Record>,
    users: Vec<UserRecord>,
    roles: Vec<RoleRecord>,
    policies: Vec<PolicyRecord>,
    buckets: Vec<BucketRecord>,
    sgs: Vec<SgRecord>,
}

impl AwsSyntheticConnector {
    /// Build the connector, pre-generating all entity data from `config.seed`.
    pub fn new(config: SyntheticConfig) -> Self {
        let mut rng = Lcg::new(config.seed);

        // VPC/subnet IDs — shared pool.
        let vpc_ids: Vec<String> = (0..4).map(|_| format!("vpc-{}", rng.hex(17))).collect();
        let subnet_ids: Vec<String> = (0..12).map(|_| format!("subnet-{}", rng.hex(17))).collect();

        // Security groups.
        let sgs: Vec<SgRecord> = (0..config.sg_count)
            .map(|i| {
                let vpc = rng.pick(&vpc_ids).clone();
                SgRecord {
                    sg_id: format!("sg-{}", rng.hex(17)),
                    name: format!("sg-{i:03}"),
                    description: *rng.pick(SG_DESCRIPTIONS),
                    vpc_id: vpc,
                    allows_all_outbound: rng.prob(0.9),
                }
            })
            .collect();

        // IAM policies (from the curated managed policy list).
        let n_policies = config.iam_role_count.min(AWS_MANAGED_POLICIES.len());
        let policies: Vec<PolicyRecord> = (0..n_policies)
            .map(|i| PolicyRecord {
                policy_name: AWS_MANAGED_POLICIES[i],
                policy_id: format!("ANPA{}", rng.hex(16).to_uppercase()),
                is_aws_managed: true,
            })
            .collect();

        // IAM roles.
        let roles: Vec<RoleRecord> = (0..config.iam_role_count)
            .map(|i| {
                let prefix = rng.pick(ROLE_PREFIXES);
                let suffix = rng.pick(ROLE_SUFFIXES);
                let is_admin = rng.prob(config.admin_role_ratio);
                // Attach 1–3 policies per role.
                let n_pol = (rng.next() as usize % 3) + 1;
                let policy_indices: Vec<usize> = (0..n_pol)
                    .map(|_| rng.next() as usize % policies.len())
                    .collect();
                RoleRecord {
                    role_name: format!("{prefix}-{suffix}-role-{i:02}"),
                    role_id: format!("AROA{}", rng.hex(16).to_uppercase()),
                    is_admin,
                    policy_indices,
                }
            })
            .collect();

        // IAM users.
        let users: Vec<UserRecord> = (0..config.iam_user_count)
            .map(|i| {
                let base_name = rng.pick(USER_FIRST);
                let username = if config.iam_user_count <= USER_FIRST.len() {
                    base_name.to_string()
                } else {
                    format!("{base_name}{i:02}")
                };
                let mfa_active = rng.prob(config.mfa_compliance);
                let active = rng.prob(0.90);
                // Assign 1–3 roles per user.
                let n_roles = (rng.next() as usize % 3) + 1;
                let role_indices: Vec<usize> = (0..n_roles)
                    .map(|_| rng.next() as usize % roles.len())
                    .collect();
                UserRecord {
                    user_id: format!("AIDA{}", rng.hex(16).to_uppercase()),
                    username,
                    mfa_active,
                    active,
                    password_last_used_days: (rng.next() % 365) as u32,
                    role_indices,
                }
            })
            .collect();

        // S3 buckets.
        let buckets: Vec<BucketRecord> = (0..config.s3_bucket_count)
            .map(|i| {
                let adj = rng.pick(BUCKET_ADJECTIVES);
                let noun = rng.pick(BUCKET_NOUNS);
                let acct = &config.account_id;
                BucketRecord {
                    name: format!("{acct}-{adj}-{noun}-{i:03}"),
                    region_idx: rng.next() as usize % REGIONS.len(),
                    public: rng.prob(config.public_bucket_ratio),
                    versioning: rng.prob(0.60),
                    logging: rng.prob(0.50),
                    encryption: rng.prob(0.85),
                }
            })
            .collect();

        // EC2 instances.
        let ec2_states_table: Vec<(&str, u8)> = EC2_STATES.iter().map(|(s, w)| (*s, *w)).collect();
        let platform_table: Vec<(&str, u8)> = PLATFORMS.iter().map(|(s, w)| (*s, *w)).collect();

        let ec2: Vec<Ec2Record> = (0..config.ec2_count)
            .map(|_| {
                let region_idx = rng.next() as usize % REGIONS.len();
                let azs = AZS[region_idx];
                let az = rng.pick(azs);
                let instance_type = rng.pick(INSTANCE_TYPES);
                let state = *rng.pick_weighted(
                    &ec2_states_table
                        .iter()
                        .map(|(s, w)| (s, *w))
                        .collect::<Vec<_>>(),
                );
                let platform = *rng.pick_weighted(
                    &platform_table
                        .iter()
                        .map(|(s, w)| (s, *w))
                        .collect::<Vec<_>>(),
                );
                let has_public = rng.prob(0.35);
                let private_ip = format!(
                    "10.{}.{}.{}",
                    rng.next() % 4,
                    rng.next() % 256,
                    rng.next() % 256
                );
                let public_ip = if has_public {
                    Some(format!(
                        "{}.{}.{}.{}",
                        18 + rng.next() % 200,
                        rng.next() % 256,
                        rng.next() % 256,
                        rng.next() % 256
                    ))
                } else {
                    None
                };
                let vpc = rng.pick(&vpc_ids).clone();
                let subnet = rng.pick(&subnet_ids).clone();
                let has_edr = rng.prob(config.edr_coverage);
                let role_idx = if rng.prob(0.70) {
                    Some(rng.next() as usize % roles.len())
                } else {
                    None
                };
                let sg_idx = rng.next() as usize % sgs.len();
                Ec2Record {
                    instance_id: format!("i-{}", rng.hex(17)),
                    instance_type,
                    state,
                    region_idx,
                    az,
                    platform,
                    private_ip,
                    public_ip,
                    monitoring: rng.prob(config.monitoring_ratio),
                    vpc_id: vpc,
                    subnet_id: subnet,
                    has_edr,
                    role_idx,
                    sg_idx,
                }
            })
            .collect();

        AwsSyntheticConnector {
            config,
            ec2,
            users,
            roles,
            policies,
            buckets,
            sgs,
        }
    }
}

#[async_trait]
impl Connector for AwsSyntheticConnector {
    fn name(&self) -> &str {
        "aws-synthetic"
    }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            // Wave 0 — all independent, run concurrently.
            step("ec2", "Emit EC2 instances").build(),
            step("iam-users", "Emit IAM users").build(),
            step("iam-roles", "Emit IAM roles").build(),
            step("iam-policies", "Emit IAM managed policies").build(),
            step("s3-buckets", "Emit S3 buckets").build(),
            step("security-groups", "Emit VPC security groups").build(),
            // Wave 1 — relationships + EDR agents reference wave-0 entity keys.
            step("edr-agents", "Emit EDR agents and PROTECTS relationships")
                .depends_on(&["ec2"])
                .build(),
            step("iam-relationships", "Emit IAM assignment relationships")
                .depends_on(&["iam-users", "iam-roles", "iam-policies"])
                .build(),
            step("ec2-relationships", "Emit EC2 role and SG relationships")
                .depends_on(&["ec2", "iam-roles", "security-groups"])
                .build(),
        ]
    }

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        match step_id {
            "ec2" => self.emit_ec2(ctx),
            "iam-users" => self.emit_iam_users(ctx),
            "iam-roles" => self.emit_iam_roles(ctx),
            "iam-policies" => self.emit_iam_policies(ctx),
            "s3-buckets" => self.emit_s3_buckets(ctx),
            "security-groups" => self.emit_security_groups(ctx),
            "edr-agents" => self.emit_edr_agents(ctx),
            "iam-relationships" => self.emit_iam_relationships(ctx),
            "ec2-relationships" => self.emit_ec2_relationships(ctx),
            other => Err(ConnectorError::UnknownStep(other.to_owned())),
        }
    }
}

// ─── Step implementations ─────────────────────────────────────────────────────

impl AwsSyntheticConnector {
    fn emit_ec2(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for r in &self.ec2 {
            let region = REGIONS[r.region_idx];
            let arn = format!(
                "arn:aws:ec2:{}:{}:instance/{}",
                region, self.config.account_id, r.instance_id
            );
            let mut e = entity("aws_ec2_instance", &r.instance_id)
                .class("Host")
                .display_name(&r.instance_id)
                .property("instanceType", r.instance_type)
                .property("state", r.state)
                .property("region", region)
                .property("availabilityZone", r.az)
                .property("platform", r.platform)
                .property("privateIpAddress", r.private_ip.as_str())
                .property("monitoring", r.monitoring)
                .property("vpcId", r.vpc_id.as_str())
                .property("subnetId", r.subnet_id.as_str())
                .property("arn", arn.as_str())
                .property("active", r.state == "running");

            if let Some(ip) = &r.public_ip {
                e = e.property("publicIpAddress", ip.as_str());
            }

            ctx.emit_entity(e)?;
        }
        Ok(())
    }

    fn emit_iam_users(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for u in &self.users {
            let arn = format!(
                "arn:aws:iam::{}:user/{}",
                self.config.account_id, u.username
            );
            ctx.emit_entity(
                entity("aws_iam_user", &u.username)
                    .class("User")
                    .display_name(&u.username)
                    .property("userId", u.user_id.as_str())
                    .property("arn", arn.as_str())
                    .property("mfaActive", u.mfa_active)
                    .property("active", u.active)
                    .property("passwordEnabled", true)
                    .property("accessKeyActive", u.active)
                    .property("passwordLastUsedDays", u.password_last_used_days as i64),
            )?;
        }
        Ok(())
    }

    fn emit_iam_roles(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for r in &self.roles {
            let arn = format!(
                "arn:aws:iam::{}:role/{}",
                self.config.account_id, r.role_name
            );
            ctx.emit_entity(
                entity("aws_iam_role", &r.role_name)
                    .class("Role")
                    .display_name(&r.role_name)
                    .property("roleId", r.role_id.as_str())
                    .property("arn", arn.as_str())
                    .property("admin", r.is_admin)
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_iam_policies(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for p in &self.policies {
            let arn = format!("arn:aws:iam::aws:policy/{}", p.policy_name);
            ctx.emit_entity(
                entity("aws_iam_policy", p.policy_name)
                    .class("AccessPolicy")
                    .display_name(p.policy_name)
                    .property("policyId", p.policy_id.as_str())
                    .property("arn", arn.as_str())
                    .property("isAWSManaged", p.is_aws_managed)
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_s3_buckets(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for b in &self.buckets {
            let region = REGIONS[b.region_idx];
            let arn = format!("arn:aws:s3:::{}", b.name);
            let acl = if b.public { "public-read" } else { "private" };
            ctx.emit_entity(
                entity("aws_s3_bucket", &b.name)
                    .class("DataStore")
                    .display_name(&b.name)
                    .property("region", region)
                    .property("public", b.public)
                    .property("versioning", b.versioning)
                    .property("logging", b.logging)
                    .property("encryption", b.encryption)
                    .property("acl", acl)
                    .property("arn", arn.as_str())
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_security_groups(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for sg in &self.sgs {
            let arn = format!(
                "arn:aws:ec2:{}:{}:security-group/{}",
                self.config.region, self.config.account_id, sg.sg_id
            );
            ctx.emit_entity(
                entity("aws_security_group", &sg.sg_id)
                    .class("Firewall")
                    .display_name(&sg.name)
                    .property("groupId", sg.sg_id.as_str())
                    .property("groupName", sg.name.as_str())
                    .property("description", sg.description)
                    .property("vpcId", sg.vpc_id.as_str())
                    .property("allowsAllOutbound", sg.allows_all_outbound)
                    .property("arn", arn.as_str())
                    .property("active", true),
            )?;
        }
        Ok(())
    }

    fn emit_edr_agents(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        // Use a secondary LCG seeded from primary seed + step offset for
        // the vendor selection, ensuring it's independent from entity generation.
        let mut rng = Lcg::new(self.config.seed.wrapping_add(0xDEADBEEF));

        for (i, ec2) in self.ec2.iter().enumerate() {
            if !ec2.has_edr {
                continue;
            }
            let vendor = rng.pick(EDR_VENDORS);
            let agent_id = format!("agent-{i:05}");
            let agent_version = format!(
                "{}.{}.{}",
                rng.next() % 8,
                rng.next() % 20,
                rng.next() % 100
            );

            // Emit the EDR agent entity.
            ctx.emit_entity(
                entity("edr_agent", &agent_id)
                    .class("Agent")
                    .display_name(&format!("{vendor} on {}", ec2.instance_id))
                    .property("vendor", vendor.as_ref())
                    .property("version", agent_version.as_str())
                    .property("active", true),
            )?;

            // Emit: edr_agent PROTECTS aws_ec2_instance.
            ctx.emit_relationship(
                relationship(&agent_id, "PROTECTS", &ec2.instance_id)
                    .from_type("edr_agent")
                    .to_type("aws_ec2_instance"),
            )?;
        }
        Ok(())
    }

    fn emit_iam_relationships(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        // aws_iam_user ASSIGNED aws_iam_role
        for user in &self.users {
            for &role_idx in &user.role_indices {
                let role = &self.roles[role_idx];
                ctx.emit_relationship(
                    relationship(&user.username, "ASSIGNED", &role.role_name)
                        .from_type("aws_iam_user")
                        .to_type("aws_iam_role"),
                )?;
            }
        }

        // aws_iam_role ASSIGNED aws_iam_policy
        for role in &self.roles {
            for &pol_idx in &role.policy_indices {
                let policy = &self.policies[pol_idx];
                ctx.emit_relationship(
                    relationship(&role.role_name, "ASSIGNED", policy.policy_name)
                        .from_type("aws_iam_role")
                        .to_type("aws_iam_policy"),
                )?;
            }
        }
        Ok(())
    }

    fn emit_ec2_relationships(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        for ec2 in &self.ec2 {
            // aws_ec2_instance USES aws_iam_role (instance profile)
            if let Some(role_idx) = ec2.role_idx {
                let role = &self.roles[role_idx];
                ctx.emit_relationship(
                    relationship(&ec2.instance_id, "USES", &role.role_name)
                        .from_type("aws_ec2_instance")
                        .to_type("aws_iam_role"),
                )?;
            }

            // aws_ec2_instance HAS aws_security_group
            let sg = &self.sgs[ec2.sg_idx];
            ctx.emit_relationship(
                relationship(&ec2.instance_id, "HAS", &sg.sg_id)
                    .from_type("aws_ec2_instance")
                    .to_type("aws_security_group"),
            )?;
        }
        Ok(())
    }
}

// ─── Convenience constructors ─────────────────────────────────────────────────

impl AwsSyntheticConnector {
    /// Pre-built "clean" scenario — all policies pass.
    /// Useful as a baseline to confirm zero violations.
    pub fn clean(scale: usize) -> Self {
        Self::new(SyntheticConfig {
            ec2_count: scale,
            iam_user_count: scale / 4,
            iam_role_count: scale / 8,
            s3_bucket_count: scale / 5,
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
            ec2_count: scale,
            iam_user_count: scale / 4,
            iam_role_count: scale / 8,
            s3_bucket_count: scale / 5,
            edr_coverage: 0.0,
            mfa_compliance: 0.0,
            public_bucket_ratio: 1.0,
            admin_role_ratio: 1.0,
            ..SyntheticConfig::default()
        })
    }

    /// Pre-built scenario matching realistic enterprise posture.
    /// ~20% violation rate across all dimensions.
    pub fn realistic(scale: usize) -> Self {
        Self::new(SyntheticConfig {
            ec2_count: scale,
            iam_user_count: scale / 3,
            iam_role_count: scale / 6,
            s3_bucket_count: scale / 4,
            edr_coverage: 0.82,
            mfa_compliance: 0.75,
            public_bucket_ratio: 0.08,
            admin_role_ratio: 0.12,
            ..SyntheticConfig::default()
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_connector() -> AwsSyntheticConnector {
        AwsSyntheticConnector::new(SyntheticConfig::default())
    }

    #[test]
    fn entity_counts_match_config() {
        let cfg = SyntheticConfig {
            ec2_count: 10,
            iam_user_count: 5,
            s3_bucket_count: 4,
            ..SyntheticConfig::default()
        };
        let c = AwsSyntheticConnector::new(cfg.clone());
        assert_eq!(c.ec2.len(), cfg.ec2_count);
        assert_eq!(c.users.len(), cfg.iam_user_count);
        assert_eq!(c.buckets.len(), cfg.s3_bucket_count);
    }

    #[test]
    fn deterministic_same_seed() {
        let a = AwsSyntheticConnector::new(SyntheticConfig {
            seed: 1,
            ..SyntheticConfig::default()
        });
        let b = AwsSyntheticConnector::new(SyntheticConfig {
            seed: 1,
            ..SyntheticConfig::default()
        });
        assert_eq!(a.ec2[0].instance_id, b.ec2[0].instance_id);
        assert_eq!(a.users[0].username, b.users[0].username);
    }

    #[test]
    fn different_seeds_produce_different_data() {
        let a = AwsSyntheticConnector::new(SyntheticConfig {
            seed: 1,
            ..SyntheticConfig::default()
        });
        let b = AwsSyntheticConnector::new(SyntheticConfig {
            seed: 2,
            ..SyntheticConfig::default()
        });
        assert_ne!(a.ec2[0].instance_id, b.ec2[0].instance_id);
    }

    #[test]
    fn edr_coverage_respected() {
        let cfg = SyntheticConfig {
            ec2_count: 200,
            edr_coverage: 0.5,
            seed: 99,
            ..SyntheticConfig::default()
        };
        let c = AwsSyntheticConnector::new(cfg);
        let covered = c.ec2.iter().filter(|e| e.has_edr).count();
        // Allow ±10% tolerance for statistical variation at n=200.
        assert!(
            (80..=120).contains(&covered),
            "edr covered={covered}, expected ~100"
        );
    }

    #[test]
    fn clean_scenario_all_covered() {
        let c = AwsSyntheticConnector::clean(20);
        assert!(c.ec2.iter().all(|e| e.has_edr));
        assert!(c.users.iter().all(|u| u.mfa_active));
        assert!(c.buckets.iter().all(|b| !b.public));
        assert!(c.roles.iter().all(|r| !r.is_admin));
    }

    #[test]
    fn worst_case_scenario_all_violated() {
        let c = AwsSyntheticConnector::worst_case(20);
        assert!(c.ec2.iter().all(|e| !e.has_edr));
        assert!(c.users.iter().all(|u| !u.mfa_active));
        assert!(c.buckets.iter().all(|b| b.public));
        assert!(c.roles.iter().all(|r| r.is_admin));
    }

    #[test]
    fn steps_form_valid_wave_structure() {
        use parallax_connect::connector::topological_waves;
        let c = default_connector();
        let waves = topological_waves(&c.steps());
        // Wave 0: 6 independent steps.
        assert_eq!(waves[0].len(), 6, "wave 0 must have 6 parallel steps");
        // Wave 1: 3 relationship steps.
        assert_eq!(waves[1].len(), 3, "wave 1 must have 3 relationship steps");
    }

    #[tokio::test]
    async fn run_emits_expected_entity_count() {
        use parallax_connect::scheduler::run_connector;
        use std::sync::Arc;

        let cfg = SyntheticConfig {
            ec2_count: 10,
            iam_user_count: 4,
            iam_role_count: 3,
            s3_bucket_count: 5,
            sg_count: 2,
            edr_coverage: 0.5,
            ..SyntheticConfig::default()
        };
        let c = Arc::new(AwsSyntheticConnector::new(cfg.clone()));
        let out = run_connector(c, "test-acct", "sync-1", None).await.unwrap();

        // Minimum entities: ec2 + users + roles + policies + buckets + sgs.
        let min_entities = cfg.ec2_count
            + cfg.iam_user_count
            + cfg.iam_role_count
            + cfg.s3_bucket_count
            + cfg.sg_count;
        assert!(
            out.entities.len() >= min_entities,
            "entities={}, expected ≥{}",
            out.entities.len(),
            min_entities
        );

        // Relationships emitted.
        assert!(
            !out.relationships.is_empty(),
            "must emit at least some relationships"
        );
    }

    #[tokio::test]
    async fn entity_types_all_present() {
        use parallax_connect::scheduler::run_connector;
        use std::collections::HashSet;
        use std::sync::Arc;

        let c = Arc::new(AwsSyntheticConnector::new(SyntheticConfig {
            ec2_count: 5,
            iam_user_count: 3,
            iam_role_count: 3,
            s3_bucket_count: 3,
            sg_count: 2,
            edr_coverage: 1.0,
            ..SyntheticConfig::default()
        }));
        let out = run_connector(c, "acct", "s1", None).await.unwrap();

        let types: HashSet<&str> = out.entities.iter().map(|e| e._type.as_str()).collect();

        for expected in &[
            "aws_ec2_instance",
            "aws_iam_user",
            "aws_iam_role",
            "aws_iam_policy",
            "aws_s3_bucket",
            "aws_security_group",
            "edr_agent",
        ] {
            assert!(types.contains(expected), "missing entity type: {expected}");
        }
    }
}
