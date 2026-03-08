# Policy Evaluation

## Loading an Evaluator

```rust
use parallax_policy::{PolicyEvaluator, load_rules_from_yaml};
use parallax_query::IndexStats;

let rules = load_rules_from_yaml(Path::new("rules/security.yaml"))?;
let stats = IndexStats::new(type_counts, class_counts, total, rel_total);
let evaluator = PolicyEvaluator::load(rules, &stats)?;
// Err if any rule contains invalid PQL (INV-P06)
```

## Running an Evaluation

### Sequential

```rust
let snap = engine.snapshot();
let graph = GraphReader::new(&snap);
let results = evaluator.evaluate_all(&graph, QueryLimits::default());
```

### Parallel (3E)

```rust
// Each rule runs on its own OS thread; results collected in definition order.
let results = evaluator.par_evaluate_all(&graph, QueryLimits::default());
```

Both methods return identical results in the same order. `par_evaluate_all`
is preferred when evaluating many rules — it uses `std::thread::scope` so
non-`'static` lifetimes (including `GraphReader<'snap>`) work correctly.

## RuleResult

```rust
pub struct RuleResult {
    pub rule_id: String,
    pub status: RuleStatus,
    pub violations: Vec<Violation>,
    pub error: Option<String>,      // set on RuleStatus::Error
    pub evaluated_at: Timestamp,
    pub duration: Duration,
}

pub enum RuleStatus {
    Pass,     // query returned 0 results
    Fail,     // query returned ≥1 results
    Error,    // rule evaluation errored (INV-P03: others still run)
    Skipped,  // rule.enabled = false
}

pub struct Violation {
    pub entity_id: EntityId,
    pub entity_type: EntityType,
    pub display_name: CompactString,
    pub details: String,
}
```

## INV-P01: Snapshot Atomicity

All rules in one `evaluate_all` / `par_evaluate_all` call read the same
snapshot. Rules see a consistent point-in-time view of the graph even if
new data is being ingested concurrently.

## INV-P02: Read-Only

Policy evaluation never modifies the graph.

## INV-P03: Error Isolation

A rule that errors during evaluation is recorded with `RuleStatus::Error` and
does **not** abort evaluation of other rules:

```rust
for result in &results {
    match result.status {
        RuleStatus::Pass    => println!("✓ {} — PASS", result.rule_id),
        RuleStatus::Fail    => println!("✗ {} — {} violations",
                                   result.rule_id, result.violations.len()),
        RuleStatus::Error   => println!("! {} — {}", result.rule_id,
                                   result.error.as_deref().unwrap_or("?")),
        RuleStatus::Skipped => println!("- {} — skipped", result.rule_id),
    }
}
```

## Performance

Each rule runs one PQL query. For N rules, `evaluate_all` makes N sequential
graph reads; `par_evaluate_all` runs all N concurrently on separate threads.

Typical throughput (100 rules, 100K entities):
- Sequential: ~1–5 seconds (depends on rule complexity and graph structure)
- Parallel: ~100–500ms (bounded by the slowest rule)
