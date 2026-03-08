# Known Relationship Verbs

Relationship verbs are a **closed set** of 15 values. A relationship with an
unknown verb is rejected at ingest time (in v0.1: warning; in v0.2: hard error).

The verb is the semantic label on a directed edge. A closed, curated set ensures
that queries like `FIND * THAT ALLOWS internet` have consistent semantics
across all connectors.

## Full Verb List

| Verb | Direction | Semantic Meaning | Example |
|---|---|---|---|
| `HAS` | A → B | Ownership or containment | `aws_account HAS aws_s3_bucket` |
| `IS` | A ↔ B | Identity or equivalence | `okta_user IS person` |
| `ASSIGNED` | A → B | Role or permission assignment | `user ASSIGNED role` |
| `ALLOWS` | A → B | Grants network or access permission | `security_group ALLOWS internet` |
| `USES` | A → B | Active dependency | `service USES database` |
| `CONTAINS` | A → B | Logical grouping (strong containment) | `aws_vpc CONTAINS aws_subnet` |
| `MANAGES` | A → B | Administrative control | `team MANAGES github_repo` |
| `CONNECTS` | A ↔ B | Network-level connectivity | `aws_vpc CONNECTS aws_vpc` |
| `PROTECTS` | A → B | Security control coverage | `edr_agent PROTECTS host` |
| `EXPLOITS` | A → B | Vulnerability exploitation | `cve EXPLOITS software_package` |
| `TRUSTS` | A → B | Trust relationship | `aws_account TRUSTS aws_account` |
| `SCANS` | A → B | Scanner coverage | `qualys_scanner SCANS host` |
| `RUNS` | A → B | Process or service execution | `host RUNS service` |
| `READS` | A → B | Data access (read) | `application READS database` |
| `WRITES` | A → B | Data access (write) | `application WRITES database` |

## Using Verbs in PQL

```sql
-- Direct verb queries
FIND host THAT RUNS service
FIND user THAT ASSIGNED role
FIND security_group THAT ALLOWS internet

-- Negated (coverage gap)
FIND host THAT !PROTECTS edr_agent
FIND service THAT !SCANS scanner

-- Multi-hop
FIND user THAT ASSIGNED role THAT ALLOWS aws_s3_bucket
FIND cve THAT EXPLOITS package THAT USES service THAT RUNS host
```

## Verb Semantics in Blast Radius

For blast radius analysis, these verbs are considered attack-relevant
by default:

```
RUNS, CONNECTS, TRUSTS, CONTAINS, HAS, USES, EXPLOITS
```

These cover the most common lateral movement patterns:
- `RUNS`: compromise a host → compromise its services
- `CONNECTS`: network path between hosts
- `TRUSTS`: cross-account / cross-system trust
- `CONTAINS`: moving from outer to inner containers
- `HAS`: ownership chain traversal
- `USES`: dependency exploitation
- `EXPLOITS`: CVE to affected system

## Verb Selection Guide

| Situation | Recommended Verb |
|---|---|
| Cloud resource ownership | `HAS` |
| IAM/RBAC assignment | `ASSIGNED` |
| Network access rules | `ALLOWS` |
| Service-to-database | `USES` or `READS`/`WRITES` |
| Host-to-service | `RUNS` |
| VPC peering | `CONNECTS` |
| Scanner-to-target | `SCANS` |
| EDR-to-host | `PROTECTS` |
| CVE-to-package | `EXPLOITS` |
| Organizational grouping | `CONTAINS` |
| Logical equivalence | `IS` |

## In Code

```rust
use parallax_core::relationship::KNOWN_VERBS;

// Validate a verb string
if KNOWN_VERBS.contains(&"RUNS") {
    let verb = RelationshipClass::new("RUNS").unwrap();
}

// Get all known verbs
println!("Known verbs: {:?}", KNOWN_VERBS);
```

## Proposing a New Verb

New verbs require a spec change and community discussion. The bar is high:
a new verb must:

1. Be semantically distinct from all existing verbs
2. Be used by at least 3 different connector types
3. Enable new query patterns not possible with existing verbs

Open an issue on GitHub to propose new verbs.
