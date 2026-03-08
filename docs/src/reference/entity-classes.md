# Known Entity Classes

Entity classes are a **closed set** defined by Parallax. An entity submitted
with an unknown class is rejected at ingest time.

The class is the broad category that enables cross-type queries:
`FIND Host` matches EC2 instances, Azure VMs, containers, and any other
entity whose class is `Host`.

## Full Class List (41 classes)

| Class | Description | Example Types |
|---|---|---|
| `Host` | Compute hosts — servers, VMs, containers | `aws_ec2_instance`, `azure_vm`, `host` |
| `User` | Human or service user accounts | `okta_user`, `aws_iam_user`, `user` |
| `DataStore` | Storage systems | `aws_s3_bucket`, `database`, `datastore` |
| `CodeRepo` | Source code repositories | `github_repo`, `gitlab_project` |
| `Firewall` | Network access control | `aws_security_group`, `firewall` |
| `AccessPolicy` | Authorization policies | `aws_iam_policy`, `access_policy` |
| `NetworkSegment` | Network segments/subnets | `aws_vpc`, `aws_subnet`, `network` |
| `Service` | Running services or processes | `service`, `microservice` |
| `Certificate` | TLS/SSL certificates | `certificate`, `tls_cert` |
| `Secret` | Secrets and tokens | `secret`, `aws_secret`, `vault_secret` |
| `Credential` | Credentials and API keys | `credential`, `api_key` |
| `Key` | Encryption keys | `aws_kms_key`, `key` |
| `Container` | Container instances | `docker_container`, `container` |
| `Pod` | Kubernetes pods | `k8s_pod`, `pod` |
| `Cluster` | Kubernetes clusters | `k8s_cluster`, `eks_cluster` |
| `Namespace` | Kubernetes namespaces | `k8s_namespace`, `namespace` |
| `Function` | Serverless functions | `aws_lambda`, `function` |
| `Queue` | Message queues | `aws_sqs_queue`, `queue` |
| `Topic` | Message topics | `aws_sns_topic`, `topic` |
| `Database` | Database instances | `aws_rds_instance`, `postgres_db` |
| `Application` | Applications or services | `application`, `web_app` |
| `Package` | Software packages | `npm_package`, `python_package` |
| `Vulnerability` | Security vulnerabilities | `cve`, `vulnerability` |
| `Identity` | Identity providers/identities | `identity`, `saml_identity` |
| `Process` | Running processes | `process`, `daemon` |
| `File` | Files and filesystems | `file`, `s3_object` |
| `Registry` | Container/package registries | `ecr_repo`, `docker_registry` |
| `Policy` | Generic policies | `policy`, `network_policy` |
| `Account` | Cloud or service accounts | `aws_account`, `gcp_project` |
| `Organization` | Organizations or tenants | `organization`, `company` |
| `Team` | Teams or groups | `team`, `department` |
| `Role` | Roles or job functions | `aws_iam_role`, `okta_group` |
| `Group` | Groups of entities | `group`, `ad_group` |
| `Device` | Physical or virtual devices | `device`, `workstation` |
| `Endpoint` | Network endpoints | `endpoint`, `api_endpoint` |
| `Scanner` | Security scanners | `scanner`, `qualys_scanner` |
| `Agent` | Security agents | `agent`, `edr_agent` |
| `Sensor` | Telemetry sensors | `sensor`, `network_tap` |
| `Ticket` | Tickets and issues | `jira_issue`, `ticket` |
| `Event` | Security events | `event`, `alert` |
| `Generic` | Catch-all for unlisted types | `generic` |

## Using Classes in PQL

```sql
-- All hosts (regardless of type: EC2, Azure VM, container, etc.)
FIND Host

-- All datastores not accessible publicly
FIND DataStore WITH public = false

-- All users without MFA
FIND User WITH mfa_enabled = false

-- Hosts with no EDR agent protecting them
FIND Host THAT !PROTECTS Agent
```

## Requesting a New Class

The class list is curated and intentionally kept small (~40 values). Before
requesting a new class, consider:

1. Can it be modeled with an existing class? (e.g., use `Generic` for truly
   novel entity types)
2. Is it used by multiple connectors, or just one? (connector-specific types
   should use an entity type, not a class)
3. Would it enable useful cross-type queries that aren't possible today?

Open an issue on GitHub to propose new classes. New classes require a spec
update and a minor version bump.

## In Code

```rust
use parallax_core::entity::KNOWN_CLASSES;

// Validate a class string
if KNOWN_CLASSES.contains(&"Host") {
    let class = EntityClass::new("Host").unwrap();
}

// Get a &[&str] of all known classes
println!("Known classes: {:?}", KNOWN_CLASSES);
```
