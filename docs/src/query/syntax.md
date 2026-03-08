# PQL Syntax Reference

## Formal Grammar (EBNF)

```ebnf
query
    = find_query
    | path_query
    | blast_query
    ;

find_query
    = "FIND" entity_filter
      ("WITH" property_expr)?
      ("THAT" traversal_chain)?
      ("GROUP" "BY" identifier)?
      ("RETURN" return_clause)?
      ("LIMIT" integer)?
    ;

path_query
    = "FIND" "SHORTEST" "PATH"
      "FROM" entity_filter ("WITH" property_expr)?
      "TO"   entity_filter ("WITH" property_expr)?
      ("DEPTH" integer)?
    ;

blast_query
    = "FIND" "BLAST" "RADIUS"
      "FROM" entity_filter ("WITH" property_expr)?
      ("DEPTH" integer)?
    ;

entity_filter
    = identifier          (* type: "host", "aws_ec2_instance" *)
    | "*"                 (* any entity *)
    ;

traversal_chain
    = traversal_step ("THAT" traversal_step)*
    ;

traversal_step
    = negation? verb entity_filter ("WITH" property_expr)?
    ;

negation = "!" ;

verb
    = "HAS" | "IS" | "ASSIGNED" | "ALLOWS" | "USES"
    | "CONTAINS" | "MANAGES" | "CONNECTS" | "PROTECTS"
    | "EXPLOITS" | "TRUSTS" | "SCANS" | "RUNS" | "READS" | "WRITES"
    ;

property_expr
    = property_or ("AND" property_or)*
    ;

property_or
    = property_cond ("OR" property_cond)*
    ;

property_cond
    = identifier comparison_op value
    | identifier "IN" "(" value_list ")"
    | identifier "LIKE" string_literal
    | identifier "EXISTS"
    | "NOT" property_cond
    ;

comparison_op = "=" | "!=" | "<" | "<=" | ">" | ">=" ;

return_clause
    = "COUNT"
    | identifier ("," identifier)*
    ;

group_by_clause
    = "GROUP" "BY" identifier
    ;

value
    = string_literal          (* 'single quoted' *)
    | integer                 (* 42 *)
    | float                   (* 3.14 *)
    | "true" | "false"
    | "null"
    ;

value_list
    = value ("," value)*
    ;

identifier
    = [a-zA-Z_][a-zA-Z0-9_]*
    ;

string_literal
    = "'" [^']* "'"           (* single quotes ONLY — no double quotes *)
    ;
```

## Important Syntax Notes

### String Literals Use Single Quotes

PQL uses **single-quoted** string literals. Double quotes are not supported.

```sql
-- Correct
FIND host WITH state = 'running'

-- Wrong (parser error)
FIND host WITH state = "running"
```

### Keywords Are Case-Sensitive

PQL keywords (`FIND`, `WITH`, `THAT`, `AND`, `RETURN`, `LIMIT`, etc.) must
be uppercase. Identifiers (entity types, property names) are case-sensitive
as well.

```sql
-- Correct
FIND host WITH state = 'running'

-- Wrong
find Host where State = 'running'
```

Note: Entity classes are `PascalCase` (`Host`, `User`, `DataStore`). Entity
types are `snake_case` (`host`, `aws_ec2_instance`).

### Negation in Traversal

The `!` negation before a verb finds entities that do **not** have a matching
neighbor. It cannot be chained.

```sql
-- Valid: hosts with no EDR agent
FIND host THAT !PROTECTS edr_agent

-- Invalid: cannot chain after negation
FIND host THAT !PROTECTS edr_agent THAT HAS service   -- syntax error
```

### RETURN Clause

Without `RETURN`, full entity objects are returned. With `RETURN COUNT`,
only the count is returned (no entity data). With `RETURN field1, field2`,
only the specified property fields are included.

```sql
-- Returns full entity objects
FIND host

-- Returns only the count
FIND host RETURN COUNT

-- Returns entities with only display_name and state properties
FIND host RETURN display_name, state
```

### DEPTH in Path Queries

`DEPTH` limits the maximum number of hops to explore. Without it, the search
is unbounded (but bounded by graph diameter in practice).

```sql
FIND SHORTEST PATH FROM user TO secret DEPTH 6
FIND BLAST RADIUS FROM host DEPTH 4
```

## Tokenization Rules

| Token | Rule |
|---|---|
| Keywords | `FIND`, `WITH`, `THAT`, `RETURN`, `LIMIT`, `AND`, `OR`, `NOT`, `IN`, `LIKE`, `EXISTS`, `GROUP`, `BY`, `SHORTEST`, `PATH`, `FROM`, `TO`, `BLAST`, `RADIUS`, `DEPTH`, `COUNT` |
| Verbs | `HAS`, `IS`, `ASSIGNED`, `ALLOWS`, `USES`, `CONTAINS`, `MANAGES`, `CONNECTS`, `PROTECTS`, `EXPLOITS`, `TRUSTS`, `SCANS`, `RUNS`, `READS`, `WRITES` |
| Identifiers | `[a-zA-Z_][a-zA-Z0-9_]*` |
| String | `'[^']*'` |
| Integer | `[0-9]+` |
| Float | `[0-9]+\.[0-9]+` |
| Boolean | `true`, `false` |
| Null | `null` |
| Operators | `=`, `!=`, `<`, `<=`, `>`, `>=` |
| Negation | `!` |
| Punctuation | `(`, `)`, `,` |
| Wildcard | `*` |
| Whitespace | Ignored |
