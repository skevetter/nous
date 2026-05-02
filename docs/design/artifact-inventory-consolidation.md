# Artifact & Inventory Consolidation

Consolidates the two parallel resource-tracking subsystems -- `artifacts` (agent-owned,
P3 feature) and `inventory` (P5 general-purpose registry) -- into a single unified
resource layer. Eliminates data duplication, inconsistent APIs, and developer confusion
about which system to use when tracking platform resources.

---

## 1. Current State

### 1.1 Artifact System (P3 -- `agents` module)

Introduced in migration `012` as part of agent relationship tracking. Artifacts are
tightly coupled to agents -- every artifact **must** have an owning `agent_id`.

#### Data model

```rust
// crates/nous-core/src/agents/mod.rs

pub enum ArtifactType {
    Worktree,
    Room,
    Schedule,
    Branch,
}

pub enum ArtifactStatus {
    Active,
    Archived,
    Deleted,
}

pub struct Artifact {
    pub id: String,
    pub agent_id: String,          // required -- FK to agents(id) ON DELETE CASCADE
    pub artifact_type: String,
    pub name: String,
    pub path: Option<String>,
    pub status: String,
    pub namespace: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_seen_at: Option<String>,
}
```

#### Database schema (migration 012)

```sql
CREATE TABLE artifacts (
    id TEXT NOT NULL PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    artifact_type TEXT NOT NULL CHECK(artifact_type IN ('worktree','room','schedule','branch')),
    name TEXT NOT NULL,
    path TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived','deleted')),
    namespace TEXT NOT NULL DEFAULT 'default',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT,
    UNIQUE(agent_id, artifact_type, name, namespace)
);
```

#### Operations

| Function | Description |
|---|---|
| `register_artifact` | Create artifact, validates agent exists and namespace matches |
| `get_artifact_by_id` | Fetch single |
| `list_artifacts` | Filter by agent_id, type, namespace, status |
| `deregister_artifact` | Hard DELETE (no soft-delete) |
| `update_artifact` | Update name or path only |

#### API endpoints

| Method | Path | Handler |
|---|---|---|
| POST | `/artifacts` | `routes::agents::register_artifact` |
| GET | `/artifacts` | `routes::agents::list_artifacts` |
| DELETE | `/artifacts/{id}` | `routes::agents::deregister_artifact` |

#### CLI commands

```
nous artifact register --agent <id> --type <type> --name <name> [--path] [--namespace]
nous artifact list [--agent] [--type] [--namespace] [--limit]
nous artifact deregister <id>
```

#### MCP tools

- `artifact_register` -- create artifact owned by agent
- `artifact_list` -- list with filters
- `artifact_deregister` -- hard delete
- `artifact_update` -- update name/path

---

### 1.2 Inventory System (P5 -- `inventory` module)

Introduced in migration `015` as a general-purpose resource registry. Inventory items
have an **optional** owner and include richer metadata (tags, JSON metadata, FTS).

#### Data model

```rust
// crates/nous-core/src/inventory/mod.rs

pub enum InventoryType {
    Worktree,
    Room,
    Schedule,
    Branch,
    File,          // not in artifacts
    DockerImage,   // not in artifacts
    Binary,        // not in artifacts
}

pub enum InventoryStatus {
    Active,
    Archived,
    Deleted,
}

pub struct InventoryItem {
    pub id: String,
    pub name: String,
    pub artifact_type: String,      // confusingly named -- uses InventoryType values
    pub owner_agent_id: Option<String>,  // optional -- can be orphaned
    pub namespace: String,
    pub path: Option<String>,
    pub status: String,
    pub metadata: Option<String>,   // JSON blob
    pub tags: String,               // JSON array
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}
```

#### Database schema (migration 015)

```sql
CREATE TABLE inventory (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    artifact_type TEXT NOT NULL CHECK(artifact_type IN ('worktree','room','schedule','branch','file','docker-image','binary')),
    owner_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
    namespace TEXT NOT NULL DEFAULT 'default',
    path TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived','deleted')),
    metadata TEXT,
    tags TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at TEXT
);
-- Plus: inventory_fts virtual table, indexes on owner, namespace+type, status, name
```

#### Operations

| Function | Description |
|---|---|
| `register_item` | Create item, optional owner, validates namespace if owner set |
| `get_item_by_id` | Fetch single |
| `list_items` | Filter by type, status, owner, namespace, orphaned |
| `update_item` | Update name, path, metadata, tags, status |
| `archive_item` | Transition active -> archived with timestamp |
| `deregister_item` | Soft-delete (status='deleted') or hard DELETE |
| `search_by_tags` | AND-semantics tag search |
| `search_fts` | Full-text search across name, type, metadata, tags |
| `transfer_ownership` | Bulk re-assign or orphan items when agent leaves |

#### API endpoints

| Method | Path | Handler |
|---|---|---|
| POST | `/inventory` | `routes::inventory::register` |
| GET | `/inventory` | `routes::inventory::list` |
| GET | `/inventory/search` | `routes::inventory::search` |
| GET | `/inventory/{id}` | `routes::inventory::get` |
| PUT | `/inventory/{id}` | `routes::inventory::update` |
| DELETE | `/inventory/{id}` | `routes::inventory::deregister` |
| POST | `/inventory/{id}/archive` | `routes::inventory::archive` |

#### CLI commands

```
nous inventory register --name <n> --type <t> [--owner] [--path] [--namespace] [--tags] [--metadata]
nous inventory list [--type] [--status] [--owner] [--orphaned] [--namespace] [--limit]
nous inventory show <id>
nous inventory update <id> [--name] [--path] [--tags] [--metadata]
nous inventory search --tag <t> [--type] [--status] [--namespace] [--limit]
nous inventory archive <id>
nous inventory deregister <id> [--hard]
```

#### MCP tools

- `inventory_register` -- create item with optional owner
- `inventory_list` -- list with filters
- `inventory_get` -- fetch by ID
- `inventory_update` -- update fields
- `inventory_search` -- tag-based search
- `inventory_archive` -- archive item
- `inventory_deregister` -- soft/hard delete

---

### 1.3 Overlap Analysis

| Dimension | `artifacts` table | `inventory` table |
|---|---|---|
| Types supported | worktree, room, schedule, branch | worktree, room, schedule, branch, file, docker-image, binary |
| Owner model | Required (`agent_id NOT NULL`, CASCADE) | Optional (`owner_agent_id`, SET NULL) |
| Metadata | None | JSON `metadata` + JSON `tags` array |
| FTS | None | `inventory_fts` virtual table |
| Soft-delete | status field exists but `deregister` is hard DELETE | Proper soft-delete with `archived_at` timestamp |
| Transfer ownership | Not supported (cascade deletes on agent removal) | `transfer_ownership()` function |
| Uniqueness | `(agent_id, artifact_type, name, namespace)` | None (name not unique) |
| Last seen tracking | `last_seen_at` column | Not present |

**Key finding**: Inventory is a strict superset of artifacts in every dimension except:
1. The mandatory-owner + cascade-delete semantics of artifacts
2. The `last_seen_at` liveness tracking on artifacts
3. The uniqueness constraint `(agent_id, artifact_type, name, namespace)`

---

## 2. Problem Statement

### 2.1 Data duplication

A worktree created for an agent may be registered in **both** the `artifacts` table
(via `nous artifact register`) and the `inventory` table (via `nous inventory register`).
There is no foreign key or cross-reference between the two tables. The same physical
resource can have two IDs, two statuses, two owners -- with no consistency guarantee.

### 2.2 Inconsistent APIs

Users face two different command surfaces for conceptually the same operation:

- `nous artifact register --agent X --type worktree --name foo`
- `nous inventory register --name foo --type worktree --owner X`

The artifact API hard-deletes; the inventory API soft-deletes. The artifact API has no
tags, no metadata, no FTS. An agent using the MCP server sees both `artifact_*` and
`inventory_*` tool families and must guess which to use.

### 2.3 Missing relationships

Artifacts cascade-delete when an agent is deregistered, which is correct for ephemeral
resources (worktrees, branches) but destructive for durable resources (rooms, schedules)
that should survive agent removal. Meanwhile, inventory items become orphaned on agent
deletion (SET NULL) but lose their provenance.

### 2.4 Developer confusion

The `InventoryItem` struct has a field named `artifact_type` (not `inventory_type`),
and the database column is also `artifact_type`. The CLI help for `nous inventory`
says "P5 artifact registry". The naming makes it unclear whether inventory IS the
artifact system or something separate.

### 2.5 Incomplete features on each side

- Artifacts lack: tags, metadata, FTS search, soft-delete, orphan detection
- Inventory lacks: liveness tracking (`last_seen_at`), uniqueness constraints,
  cascade semantics for ephemeral resources

---

## 3. Target State

### 3.1 Unified resource model

A single `resources` table replaces both `artifacts` and `inventory`. The new model
takes the richer schema of inventory and adds the missing artifact capabilities.

Core abstraction: a **Resource** is any trackable entity in the platform, optionally
owned by an agent, classified by type, with rich metadata and lifecycle management.

### 3.2 Unified data model

```rust
// crates/nous-core/src/resources/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceType {
    Worktree,
    Room,
    Schedule,
    Branch,
    File,
    DockerImage,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceStatus {
    Active,
    Archived,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnershipPolicy {
    /// Resource is deleted when owning agent is deregistered (like current artifacts)
    CascadeDelete,
    /// Resource ownership is set to NULL when agent is deregistered (like current inventory)
    Orphan,
    /// Resource is transferred to the agent's parent when agent is deregistered
    TransferToParent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: String,
    pub name: String,
    pub resource_type: String,
    pub owner_agent_id: Option<String>,
    pub namespace: String,
    pub path: Option<String>,
    pub status: String,
    pub metadata: Option<String>,         // JSON blob
    pub tags: String,                     // JSON array
    pub ownership_policy: String,         // cascade-delete | orphan | transfer-to-parent
    pub last_seen_at: Option<String>,     // liveness tracking from artifacts
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}
```

### 3.3 Database schema

```sql
CREATE TABLE resources (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    resource_type TEXT NOT NULL CHECK(resource_type IN (
        'worktree','room','schedule','branch','file','docker-image','binary'
    )),
    owner_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
    namespace TEXT NOT NULL DEFAULT 'default',
    path TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived','deleted')),
    metadata TEXT,
    tags TEXT NOT NULL DEFAULT '[]',
    ownership_policy TEXT NOT NULL DEFAULT 'orphan' CHECK(ownership_policy IN (
        'cascade-delete','orphan','transfer-to-parent'
    )),
    last_seen_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at TEXT,
    -- Preserve the uniqueness semantics from artifacts for owned resources
    UNIQUE(owner_agent_id, resource_type, name, namespace)
);

CREATE INDEX idx_resources_owner ON resources(owner_agent_id);
CREATE INDEX idx_resources_namespace_type ON resources(namespace, resource_type);
CREATE INDEX idx_resources_status ON resources(status);
CREATE INDEX idx_resources_name ON resources(name);
CREATE INDEX idx_resources_last_seen ON resources(last_seen_at) WHERE last_seen_at IS NOT NULL;

CREATE TRIGGER resources_au AFTER UPDATE ON resources
    WHEN NEW.updated_at = OLD.updated_at
    BEGIN UPDATE resources SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id; END;

CREATE VIRTUAL TABLE resources_fts USING fts5(
    content, content_rowid='rowid', tokenize='porter unicode61'
);

-- FTS triggers (insert, update, delete) matching inventory_fts pattern
```

### 3.4 Ownership policy enforcement

When an agent is deregistered, a new pre-deletion hook processes resources by policy:

```rust
pub async fn handle_agent_deregistration(pool: &SqlitePool, agent_id: &str) -> Result<(), NousError> {
    // 1. Hard-delete resources with cascade-delete policy
    sqlx::query("DELETE FROM resources WHERE owner_agent_id = ? AND ownership_policy = 'cascade-delete'")
        .bind(agent_id).execute(pool).await?;

    // 2. Transfer resources with transfer-to-parent policy
    let parent_id = get_parent_agent_id(pool, agent_id).await?;
    if let Some(parent) = parent_id {
        sqlx::query("UPDATE resources SET owner_agent_id = ? WHERE owner_agent_id = ? AND ownership_policy = 'transfer-to-parent'")
            .bind(&parent).bind(agent_id).execute(pool).await?;
    } else {
        // No parent -- fall back to orphan
        sqlx::query("UPDATE resources SET owner_agent_id = NULL, status = 'archived', archived_at = ? WHERE owner_agent_id = ? AND ownership_policy = 'transfer-to-parent'")
            .bind(&now()).bind(agent_id).execute(pool).await?;
    }

    // 3. Orphan remaining resources (policy = 'orphan')
    sqlx::query("UPDATE resources SET owner_agent_id = NULL WHERE owner_agent_id = ? AND ownership_policy = 'orphan'")
        .bind(agent_id).execute(pool).await?;

    Ok(())
}
```

### 3.5 Mapping from existing systems

| Source | Maps to |
|---|---|
| `artifacts` with type=worktree/branch | Resource with `ownership_policy = cascade-delete` |
| `artifacts` with type=room/schedule | Resource with `ownership_policy = orphan` |
| `inventory` items with owner | Resource with `ownership_policy = orphan` |
| `inventory` items without owner | Resource with `owner_agent_id = NULL, ownership_policy = orphan` |

---

## 4. Architecture

### 4.1 New crate structure

```
crates/nous-core/src/
    resources/
        mod.rs         -- Resource, ResourceType, ResourceStatus, OwnershipPolicy, CRUD ops
        migration.rs   -- data migration helpers (artifacts + inventory -> resources)
    inventory/         -- REMOVED (replaced by resources)
    agents/
        mod.rs         -- Artifact types/ops REMOVED, agents module focuses on agent-only concerns
```

### 4.2 New types

```rust
// crates/nous-core/src/resources/mod.rs

pub enum ResourceType { Worktree, Room, Schedule, Branch, File, DockerImage, Binary }
pub enum ResourceStatus { Active, Archived, Deleted }
pub enum OwnershipPolicy { CascadeDelete, Orphan, TransferToParent }

pub struct Resource { /* see section 3.2 */ }

pub struct RegisterResourceRequest {
    pub name: String,
    pub resource_type: ResourceType,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
    pub ownership_policy: Option<OwnershipPolicy>,
}

pub struct UpdateResourceRequest {
    pub id: String,
    pub name: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<ResourceStatus>,
    pub ownership_policy: Option<OwnershipPolicy>,
}

pub struct ListResourcesFilter {
    pub resource_type: Option<ResourceType>,
    pub status: Option<ResourceStatus>,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub orphaned: Option<bool>,
    pub ownership_policy: Option<OwnershipPolicy>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub struct SearchResourcesRequest {
    pub tags: Vec<String>,
    pub resource_type: Option<ResourceType>,
    pub status: Option<ResourceStatus>,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}
```

### 4.3 API changes

#### New endpoints (replace both `/artifacts` and `/inventory`)

| Method | Path | Description |
|---|---|---|
| POST | `/resources` | Register a resource |
| GET | `/resources` | List with filters |
| GET | `/resources/search` | Tag-based search |
| GET | `/resources/{id}` | Get by ID |
| PUT | `/resources/{id}` | Update fields |
| POST | `/resources/{id}/archive` | Archive |
| DELETE | `/resources/{id}` | Deregister (soft by default, `?hard=true` for hard) |
| POST | `/resources/{id}/heartbeat` | Update `last_seen_at` |
| POST | `/resources/transfer` | Bulk transfer ownership |

#### Deprecated endpoints (kept during transition)

| Method | Path | Behavior |
|---|---|---|
| POST/GET | `/artifacts` | Proxies to `/resources` with `ownership_policy=cascade-delete` |
| DELETE | `/artifacts/{id}` | Proxies to `DELETE /resources/{id}?hard=true` |
| POST/GET | `/inventory` | Proxies to `/resources` |
| * | `/inventory/*` | Proxies to corresponding `/resources/*` |

### 4.4 CLI changes

#### New primary command

```
nous resource register --name <n> --type <t> [--owner] [--path] [--namespace] [--tags] [--metadata] [--policy cascade-delete|orphan|transfer-to-parent]
nous resource list [--type] [--status] [--owner] [--orphaned] [--namespace] [--policy] [--limit]
nous resource show <id>
nous resource update <id> [--name] [--path] [--tags] [--metadata] [--policy]
nous resource search --tag <t> [--type] [--status] [--namespace] [--limit]
nous resource archive <id>
nous resource deregister <id> [--hard]
nous resource heartbeat <id>
nous resource transfer --from <agent-id> [--to <agent-id>]
```

#### Deprecated commands (kept during transition)

```
nous artifact *   -- prints deprecation warning, delegates to `nous resource`
nous inventory *  -- prints deprecation warning, delegates to `nous resource`
```

### 4.5 MCP tool changes

#### New tools

- `resource_register` -- replaces both `artifact_register` and `inventory_register`
- `resource_list` -- replaces `artifact_list` and `inventory_list`
- `resource_get` -- replaces `inventory_get`
- `resource_update` -- replaces `artifact_update` and `inventory_update`
- `resource_search` -- replaces `inventory_search`
- `resource_archive` -- replaces `inventory_archive`
- `resource_deregister` -- replaces `artifact_deregister` and `inventory_deregister`
- `resource_heartbeat` -- new (liveness tracking)
- `resource_transfer` -- new (replaces `transfer_ownership`)

#### Deprecated tools (kept during transition)

All `artifact_*` and `inventory_*` tools remain functional but internally delegate to
the `resource_*` implementations. They are removed in a later phase.

---

## 5. Implementation Plan

### Phase 1: Add resources module (non-breaking)

1. Create `crates/nous-core/src/resources/mod.rs` with new types and all CRUD operations
2. Add migration `027` creating the `resources` table and `resources_fts`
3. Add `/resources` API routes in daemon
4. Add `nous resource` CLI subcommand
5. Add `resource_*` MCP tools
6. Write unit + integration tests for the new module

**Result**: Both old and new systems coexist. No existing functionality breaks.

### Phase 2: Data migration

1. Add migration `028` that copies data from `artifacts` and `inventory` into `resources`:
   ```sql
   -- Migrate artifacts
   INSERT INTO resources (id, name, resource_type, owner_agent_id, namespace, path, status, metadata, tags, ownership_policy, last_seen_at, created_at, updated_at)
   SELECT id, name, artifact_type, agent_id, namespace, path, status, NULL, '[]',
       CASE artifact_type
           WHEN 'worktree' THEN 'cascade-delete'
           WHEN 'branch' THEN 'cascade-delete'
           ELSE 'orphan'
       END,
       last_seen_at, created_at, updated_at
   FROM artifacts;

   -- Migrate inventory (skip items already migrated via artifacts with matching name/type/owner)
   INSERT OR IGNORE INTO resources (id, name, resource_type, owner_agent_id, namespace, path, status, metadata, tags, ownership_policy, created_at, updated_at, archived_at)
   SELECT id, name, artifact_type, owner_agent_id, namespace, path, status, metadata, tags, 'orphan', created_at, updated_at, archived_at
   FROM inventory;
   ```
2. Update `deregister_agent()` to call `handle_agent_deregistration()` before deleting

**Result**: All existing data is available through the new API.

### Phase 3: Wire deprecated endpoints to resources

1. Rewrite `routes::agents::register_artifact`, `list_artifacts`, `deregister_artifact`
   to delegate to `resources` module internally
2. Rewrite `routes::inventory::*` to delegate to `resources` module
3. Rewrite CLI `nous artifact *` and `nous inventory *` to print deprecation warnings
   and call `resources` module functions
4. Rewrite MCP `artifact_*` and `inventory_*` tools to delegate to `resource_*` logic

**Result**: All code paths go through the unified resources module. Old tables are no
longer written to.

### Phase 4: Remove legacy (breaking)

1. Remove `crates/nous-core/src/inventory/` module
2. Remove artifact types/operations from `crates/nous-core/src/agents/mod.rs`
3. Remove `/artifacts` and `/inventory` API routes
4. Remove `nous artifact` and `nous inventory` CLI commands
5. Remove `artifact_*` and `inventory_*` MCP tools
6. Add migration dropping `artifacts` and `inventory` tables
7. Update MCP `--tools` prefix filter: replace `artifact` and `inventory` with `resource`

**Result**: Clean single-system architecture. This is a major version bump.

### Backwards compatibility

- **Phase 1-3**: Fully backwards compatible. Existing clients continue working.
- **Phase 4**: Breaking change. Requires coordination with MCP clients and scripts.
  Gate behind a feature flag or major version bump.

---

## 6. Testing Strategy

### Unit tests (`crates/nous-core/src/resources/`)

- All CRUD operations (register, get, list, update, archive, deregister)
- Ownership policy enforcement on agent deregistration
- Tag search with AND semantics
- FTS search
- Transfer ownership (to agent, orphan)
- Heartbeat / last_seen_at updates
- Namespace isolation
- Uniqueness constraint `(owner_agent_id, resource_type, name, namespace)`
- Edge cases: empty names, invalid metadata JSON, non-existent owners

### Integration tests (`crates/nous-daemon/tests/`)

- Full HTTP lifecycle through `/resources` endpoints
- Deprecation proxy: `/artifacts` and `/inventory` route through resources correctly
- MCP tool calls for `resource_*` family
- Agent deregistration triggers correct ownership policy behavior

### Migration tests

- Fresh DB gets both old tables and new `resources` table
- Migration 028 correctly copies data from artifacts + inventory
- Duplicate detection: items in both old tables produce single resource entry
- Round-trip: data accessible via old and new APIs after migration

### Compatibility tests (Phase 3)

- Existing MCP clients using `artifact_register` still work
- Existing scripts using `nous inventory list` still work
- Deprecation warnings are emitted but do not break output parsing (stderr only)

---

## 7. Open Questions

1. **Uniqueness constraint scope**: The current constraint is
   `(owner_agent_id, resource_type, name, namespace)`. Should unowned resources
   (NULL owner) also have a uniqueness constraint on `(resource_type, name, namespace)`?
   SQLite treats NULLs as distinct in UNIQUE constraints, so unowned resources would
   bypass this check unless we add a partial index.

2. **Default ownership policy per type**: Should `ResourceType` have a default policy
   (e.g., worktree/branch default to `cascade-delete`, room/schedule default to `orphan`)?
   This would reduce verbosity in the common case.

3. **Version / content-addressable tracking**: Should resources support immutable versions
   (e.g., docker image digests, binary checksums)? This could be a `content_hash` column
   added later.

4. **Cross-namespace references**: Can a resource in namespace A be owned by an agent in
   namespace B? Currently both systems reject this. The new system should preserve this
   constraint.
