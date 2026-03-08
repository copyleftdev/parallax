# PQL Examples

Real-world query patterns organized by use case.

## Asset Discovery

```sql
-- All hosts (any type: physical, VM, container)
FIND Host

-- All running cloud instances
FIND host WITH state = 'running'

-- All hosts in a specific region
FIND host WITH region = 'us-east-1'

-- All public-facing S3 buckets
FIND aws_s3_bucket WITH public = true

-- Count all entities by class
FIND Host RETURN COUNT
FIND User RETURN COUNT
FIND DataStore RETURN COUNT
```

## Vulnerability Analysis

```sql
-- Find all vulnerable packages
FIND package WITH has_vulnerability = true

-- Find hosts running vulnerable software
FIND host THAT RUNS package WITH has_vulnerability = true

-- Packages that exploit a specific CVE class
FIND package THAT EXPLOITS vulnerability WITH severity = 'critical'
```

## Access and Privilege

```sql
-- All users with admin roles
FIND user THAT ASSIGNED role WITH name = 'Administrator'

-- All policies that allow S3 access
FIND policy THAT ALLOWS aws_s3_bucket

-- Users who can access a specific bucket (indirect — via roles and policies)
FIND user THAT ASSIGNED role THAT ALLOWS aws_s3_bucket WITH public = false

-- Service accounts with broad permissions
FIND service_account THAT ASSIGNED role WITH permissions = 'all'
```

## Network Connectivity

```sql
-- All services exposed to the internet
FIND service THAT CONNECTS internet

-- Hosts that allow inbound traffic from any IP
FIND firewall WITH allow_all = true THAT PROTECTS host

-- Internal services with no firewall
FIND service THAT !PROTECTS firewall
```

## Security Coverage Gaps

```sql
-- Hosts with no EDR agent
FIND host THAT !PROTECTS edr_agent

-- Hosts never scanned
FIND host THAT !SCANS scanner

-- Services with no firewall protection
FIND service THAT !PROTECTS firewall

-- Databases with no backup
FIND Database THAT !HAS backup_job
```

## Container and Cloud Native

```sql
-- All pods running as root
FIND pod WITH run_as_root = true

-- Containers with no resource limits
FIND container WITH cpu_limit = null

-- Clusters with nodes that have outdated Kubernetes versions
FIND cluster THAT CONTAINS node WITH k8s_version < '1.28.0'

-- Namespaces with privileged pods
FIND namespace THAT CONTAINS pod WITH privileged = true
```

## Path and Reachability

```sql
-- Shortest path from a user to a secret
FIND SHORTEST PATH
  FROM user WITH email = 'alice@corp.com'
  TO secret WITH name = 'prod-db-password'

-- Shortest path between two networks
FIND SHORTEST PATH
  FROM aws_vpc WITH _key = 'vpc-prod'
  TO aws_vpc WITH _key = 'vpc-dev'
  DEPTH 10

-- Is there any connection between these systems?
FIND SHORTEST PATH
  FROM host WITH hostname = 'web-01'
  TO DataStore WITH name = 'customer-data'
```

## Blast Radius

```sql
-- If web-01 is compromised, what's at risk?
FIND BLAST RADIUS FROM host WITH _key = 'web-01' DEPTH 4

-- Blast radius from a compromised credential
FIND BLAST RADIUS FROM credential WITH name = 'prod-api-key' DEPTH 5

-- Blast radius from a vulnerable package
FIND BLAST RADIUS FROM package WITH cve = 'CVE-2024-1234' DEPTH 3
```

## Multi-hop Traversal

```sql
-- Complete access chain: user → role → policy → resource
FIND user
  THAT ASSIGNED role
  THAT ALLOWS policy
  THAT USES aws_s3_bucket

-- Application stack: user → app → service → database
FIND user
  THAT USES application
  THAT CONNECTS service
  THAT USES database WITH environment = 'production'

-- Supply chain: package → application → host
FIND package WITH has_vulnerability = true
  THAT USES application
  THAT RUNS host WITH environment = 'production'
```

## Compliance Queries

```sql
-- CIS: All admin users (should be minimal)
FIND user THAT ASSIGNED role WITH admin = true RETURN COUNT

-- PCI-DSS: Services that touch cardholder data
FIND service THAT USES database WITH contains_card_data = true

-- NIST: Hosts without encryption at rest
FIND host WITH encryption_at_rest = false

-- All external-facing services without WAF
FIND service WITH external = true THAT !PROTECTS waf
```
