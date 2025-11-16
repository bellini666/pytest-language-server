# Code Action Testing Guide

## Changes Made

### 1. Updated Code Action Provider Capability
**Problem:** The LSP server was registering code actions using `CodeActionProviderCapability::Simple(true)`, which doesn't explicitly declare which action kinds are supported.

**Solution:** Changed to `CodeActionProviderCapability::Options` with explicit `CodeActionKind::QUICKFIX` declaration.

**Location:** `src/main.rs:52`

```rust
code_action_provider: Some(CodeActionProviderCapability::Options(
    CodeActionOptions {
        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
        work_done_progress_options: WorkDoneProgressOptions {
            work_done_progress: None,
        },
        resolve_provider: None,
    },
)),
```

### 2. Added Filtering by `context.only`
**Problem:** The code action handler wasn't respecting the client's `only` filter parameter, which is used to request specific kinds of actions.

**Solution:** Added check at the beginning of `code_action()` to filter out requests that don't match our supported action kinds.

**Location:** `src/main.rs:456-467`

### 3. Enhanced Debug Logging
Added extensive logging throughout the code action handler to help debug issues:
- Log when code actions are requested
- Log the `context.only` filter if present
- Log how many undeclared fixtures were found
- Log each diagnostic being processed
- Log when a matching fixture is found
- Log when a code action is created
- Log the final count of actions returned

## How to Test

### 1. Build the Server
```bash
cargo build --release
# Binary will be at: target/release/pytest-language-server
```

### 2. Create a Test File
Create a Python test file with undeclared fixtures:

```python
# conftest.py
import pytest

@pytest.fixture
def my_fixture():
    return 42

# test_example.py
def test_undeclared():
    result = my_fixture + 1  # Using fixture without declaring it
    assert result == 43
```

### 3. Configure Your Editor

#### For Zed
Update your settings to use the debug binary:
```json
{
  "lsp": {
    "pytest-lsp": {
      "binary": {
        "path": "/path/to/pytest-language-server/target/release/pytest-language-server"
      }
    }
  }
}
```

#### For VSCode
Update your settings to point to the binary and enable LSP tracing:
```json
{
  "pytest-lsp.server.path": "/path/to/pytest-language-server/target/release/pytest-language-server",
  "pytest-lsp.trace.server": "verbose"
}
```

### 4. Test with Debug Logging
Run your editor with debug logging enabled:

```bash
RUST_LOG=info zed  # or your editor
```

Watch the logs (usually in `~/.local/share/zed/logs/` or similar) for:
- "code_action request" messages
- "Found X undeclared fixtures in file"
- "Created code action: Add 'fixture_name' fixture parameter"
- "Returning N code actions"

### 5. Verify Code Actions Appear

1. Open the test file
2. You should see a warning squiggle under `my_fixture` on line 2
3. Place your cursor on the warning
4. Trigger the code actions menu (usually Cmd+. or Ctrl+.)
5. You should see: "Add 'my_fixture' fixture parameter"
6. Selecting it should modify the function to: `def test_undeclared(my_fixture):`

## Expected Behavior

### Diagnostics
- **When:** File is opened or modified
- **What:** Warning appears on undeclared fixture names
- **Message:** "Fixture 'fixture_name' is used but not declared as a parameter"
- **Code:** "undeclared-fixture"
- **Source:** "pytest-lsp"

### Code Actions
- **When:** Cursor is on a diagnostic warning
- **What:** Quick fix appears in code action menu
- **Title:** "Add 'fixture_name' fixture parameter"
- **Kind:** QuickFix
- **Preferred:** Yes
- **Effect:** Adds the fixture name as a parameter to the function

## Troubleshooting

### Code Actions Not Appearing

1. **Check diagnostics are published:**
   - Look for "Publishing N diagnostics" in logs
   - Verify diagnostic appears in editor

2. **Check code_action requests are being made:**
   - Look for "code_action request" in logs
   - Check `diagnostics=N` count is > 0

3. **Check the `only` filter:**
   - Log should show `only=Some([...])` or `only=None`
   - If filtered out, you'll see "Code action request filtered out"

4. **Check undeclared fixtures are found:**
   - Look for "Found N undeclared fixtures in file"
   - If 0, the fixture detection may not be working

5. **Check matching logic:**
   - Look for "Looking for undeclared fixture at line=X, char=Y"
   - Look for "Found matching fixture: fixture_name"

6. **Check editor support:**
   - Some editors may not support code actions on diagnostics
   - Try a different editor (VSCode, Neovim with LSP, Zed)

### Still Not Working?

Enable trace-level logging for maximum detail:
```bash
RUST_LOG=trace,tower_lsp=debug zed
```

Check the LSP protocol messages to see:
- If `initialize` response includes code action capability
- If `textDocument/codeAction` requests are being sent
- What diagnostics are in the request context
- What actions are returned in the response

## Testing Checklist

- [ ] Server builds without errors
- [ ] All 52 tests pass
- [ ] Diagnostic appears on undeclared fixture
- [ ] Code action menu can be triggered
- [ ] "Add fixture parameter" action appears
- [ ] Executing action adds parameter to function
- [ ] Works with functions that have no parameters
- [ ] Works with functions that already have parameters (adds comma)
- [ ] Multiple undeclared fixtures each have their own action
- [ ] Debug logs show expected messages

## Known Limitations

1. **Single-line function signatures only:** The current implementation only handles function signatures on a single line. Multi-line signatures are not yet supported for code actions (though they are detected correctly).

2. **No async function handling:** Async functions work for detection but the text insertion logic may need adjustment.

3. **Simple parameter insertion:** The logic assumes a simple `)` pattern. Complex signatures with type hints or default values may not insert in the optimal position.

## Next Steps

If code actions still don't appear after these changes:

1. **Test with a different LSP client** to isolate whether it's a server or client issue
2. **Capture LSP protocol traces** to see the exact JSON-RPC messages
3. **Compare with other LSP servers** that successfully show code actions
4. **Check LSP spec compliance** for any missed requirements

The changes made address the two most common reasons code actions don't appear:
- Not declaring supported action kinds in capabilities
- Not respecting the `context.only` filter

If these don't resolve the issue, it's likely a client-side configuration or compatibility issue.
