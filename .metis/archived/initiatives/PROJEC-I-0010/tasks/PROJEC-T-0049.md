---
id: implement-crates-io-api-client
level: task
title: "Implement crates.io API client (fetch metadata, download source tarball)"
short_code: "PROJEC-T-0049"
created_at: 2026-01-19T14:24:27.065216+00:00
updated_at: 2026-01-19T20:42:26.515671+00:00
parent: PROJEC-I-0010
blocked_by: []
archived: true

tags:
  - "#task"
  - "#phase/completed"


exit_criteria_met: false
initiative_id: PROJEC-I-0010
---

# Implement crates.io API client (fetch metadata, download source tarball)

*This template includes sections for various types of tasks. Delete sections that don't apply to your specific use case.*

## Parent Initiative **[CONDITIONAL: Assigned Task]**

[[PROJEC-I-0010]]

## Objective **[REQUIRED]**

{Clear statement of what this task accomplishes}

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

## Acceptance Criteria **[REQUIRED]**

- [ ] {Specific, testable requirement 1}
- [ ] {Specific, testable requirement 2}
- [ ] {Specific, testable requirement 3}

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

## Implementation Notes **[CONDITIONAL: Technical Task]**

{Keep for technical tasks, delete for non-technical. Technical details, approach, or important considerations}

### Technical Approach
{How this will be implemented}

### Dependencies
{Other tasks or systems this depends on}

### Risk Considerations
{Technical risks and mitigation strategies}

## Status Updates **[REQUIRED]**

### 2026-01-19: Implementation Complete

Implemented crates.io API client for fetching Rust crate metadata and downloading source tarballs.

**New Module:** `crates/muninn-graph/src/registry/`
- `mod.rs` - Registry module that will contain both crates.io and PyPI clients
- `crates_io.rs` - Complete crates.io API client

**CratesIoClient API:**
- `new()` / `with_user_agent()` - Create client with proper user agent
- `get_crate(name)` - Fetch full crate metadata including all versions
- `get_latest_version(name)` - Get latest non-yanked version
- `get_version(name, version)` - Get specific version
- `download_source(name, version, dir)` - Download and extract source tarball
- `download_latest(name, dir)` - Convenience for latest version download

**Data Types:**
- `CrateResponse` - Full API response with crate info and versions
- `CrateInfo` - Crate metadata (name, description, URLs, downloads)
- `CrateVersion` - Version info (num, yanked, checksum, MSRV, license)
- `VersionLinks` - API links for dependencies
- `CratesIoError` - Comprehensive error enum

**Dependencies Added:**
- `reqwest` (0.12) - HTTP client with JSON/blocking support
- `flate2` (1.0) - Gzip decompression
- `tar` (0.4) - Tarball extraction
- `serde_json` (workspace) - JSON parsing
- `tokio` (workspace) - Async runtime

**Tests:**
- `test_client_creation` - Basic instantiation
- `test_client_custom_user_agent` - Custom user agent
- `test_get_crate` - API metadata fetch (network, ignored by default)
- `test_get_latest_version` - Latest version lookup (network)
- `test_get_nonexistent_crate` - 404 handling (network)
- `test_download_source` - Tarball download + extraction (network)
- `test_download_latest` - Convenience method (network)

**Test Results:** 
- 2 unit tests pass
- 5 network tests pass (run with `--ignored`)

**Files Created:**
- `crates/muninn-graph/src/registry/mod.rs`
- `crates/muninn-graph/src/registry/crates_io.rs`

**Files Modified:**
- `crates/muninn-graph/Cargo.toml` (added dependencies)
- `crates/muninn-graph/src/lib.rs` (added registry module)