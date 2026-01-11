---
id: implement-file-watcher-with
level: task
title: "Implement file watcher with debounced events"
short_code: "PROJEC-T-0007"
created_at: 2026-01-08T03:02:57.579272+00:00
updated_at: 2026-01-08T18:28:37.438894+00:00
parent: PROJEC-I-0001
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
strategy_id: NULL
initiative_id: PROJEC-I-0001
---

# Implement file watcher with debounced events

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0001]]

## Objective

Implement file system watching for continuous incremental graph updates. When source files change, automatically trigger rebuilds of affected nodes and edges.

## Backlog Item Details **[CONDITIONAL: Backlog Item]**

{Delete this section when task is assigned to an initiative}

### Type
- [ ] Bug - Production issue that needs fixing
- [ ] Feature - New functionality or enhancement  
- [ ] Tech Debt - Code improvement or refactoring
- [ ] Chore - Maintenance or setup work

### Priority
- [ ] P0 - Critical (blocks users/revenue)
- [ ] P1 - High (important for user experience)
- [ ] P2 - Medium (nice to have)
- [ ] P3 - Low (when time permits)

### Impact Assessment **[CONDITIONAL: Bug]**
- **Affected Users**: {Number/percentage of users affected}
- **Reproduction Steps**: 
  1. {Step 1}
  2. {Step 2}
  3. {Step 3}
- **Expected vs Actual**: {What should happen vs what happens}

### Business Justification **[CONDITIONAL: Feature]**
- **User Value**: {Why users need this}
- **Business Value**: {Impact on metrics/revenue}
- **Effort Estimate**: {Rough size - S/M/L/XL}

### Technical Debt Impact **[CONDITIONAL: Tech Debt]**
- **Current Problems**: {What's difficult/slow/buggy now}
- **Benefits of Fixing**: {What improves after refactoring}
- **Risk Assessment**: {Risks of not addressing this}

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

## Acceptance Criteria

- [ ] Add `notify` crate dependency
- [ ] `FileWatcher` struct with async event stream
- [ ] Filter events to supported file extensions only
- [ ] Debounce rapid events (300ms buffer)
- [ ] Handle create, modify, delete events
- [ ] Integrate with GraphBuilder for automatic rebuilds
- [ ] Respect .gitignore patterns (ignore target/, node_modules/, etc.)
- [ ] Graceful shutdown on drop
- [ ] Integration test: modify file, verify graph updates

## Test Cases **[CONDITIONAL: Testing Task]**

{Delete unless this is a testing task}

### Test Case 1: {Test Case Name}
- **Test ID**: TC-001
- **Preconditions**: {What must be true before testing}
- **Steps**: 
  1. {Step 1}
  2. {Step 2}
  3. {Step 3}
- **Expected Results**: {What should happen}
- **Actual Results**: {To be filled during execution}
- **Status**: {Pass/Fail/Blocked}

### Test Case 2: {Test Case Name}
- **Test ID**: TC-002
- **Preconditions**: {What must be true before testing}
- **Steps**: 
  1. {Step 1}
  2. {Step 2}
- **Expected Results**: {What should happen}
- **Actual Results**: {To be filled during execution}
- **Status**: {Pass/Fail/Blocked}

## Documentation Sections **[CONDITIONAL: Documentation Task]**

{Delete unless this is a documentation task}

### User Guide Content
- **Feature Description**: {What this feature does and why it's useful}
- **Prerequisites**: {What users need before using this feature}
- **Step-by-Step Instructions**:
  1. {Step 1 with screenshots/examples}
  2. {Step 2 with screenshots/examples}
  3. {Step 3 with screenshots/examples}

### Troubleshooting Guide
- **Common Issue 1**: {Problem description and solution}
- **Common Issue 2**: {Problem description and solution}
- **Error Messages**: {List of error messages and what they mean}

### API Documentation **[CONDITIONAL: API Documentation]**
- **Endpoint**: {API endpoint description}
- **Parameters**: {Required and optional parameters}
- **Example Request**: {Code example}
- **Example Response**: {Expected response format}

## Implementation Notes

### Location
`crates/muninn-graph/src/watcher.rs`

### Dependencies to Add
```toml
[dependencies]
notify = "6"
notify-debouncer-mini = "0.4"
ignore = "0.4"  # For .gitignore support
```

### FileWatcher Interface

```rust
pub struct FileWatcher {
    debouncer: Debouncer<RecommendedWatcher>,
    rx: Receiver<DebouncedEvent>,
}

pub enum FileEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
}

impl FileWatcher {
    pub fn new(root: &Path) -> Result<Self>;
    pub async fn next_event(&mut self) -> Option<FileEvent>;
    pub fn watch_path(&mut self, path: &Path) -> Result<()>;
    pub fn unwatch_path(&mut self, path: &Path) -> Result<()>;
}
```

### Integration with GraphBuilder

```rust
pub async fn watch_and_build(
    watcher: &mut FileWatcher,
    builder: &mut GraphBuilder,
) -> Result<()> {
    while let Some(event) = watcher.next_event().await {
        match event {
            FileEvent::Created(p) | FileEvent::Modified(p) => {
                builder.rebuild_file(&p)?;
            }
            FileEvent::Deleted(p) => {
                builder.delete_file(&p)?;
            }
        }
    }
    Ok(())
}
```

### Ignore Patterns
Use `ignore` crate to respect:
- `.gitignore`
- `.muninnignore` (custom)
- Built-in ignores: `target/`, `node_modules/`, `.git/`

### Reference
- narsil-mcp `persist.rs` AsyncFileWatcher pattern

### Dependencies
- Depends on: PROJEC-T-0006 (GraphBuilder)

## Status Updates

*To be added during implementation*