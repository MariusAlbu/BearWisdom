# VBA Extraction Rules

## Classification
- **Type**: scripting / macro
- **Paradigms**: procedural, event-driven, OOP (class modules)
- **Key features**: case-insensitive keywords, Sub/Function distinction, Property Get/Let/Set accessors, class modules identified by `Attribute VB_Name`, line continuation with ` _`, `Option Explicit` scope guard, COM object model integration

## Important Grammar Note
VBA has no tree-sitter grammar. Extraction is performed entirely by a **case-insensitive line scanner** (`extract.rs`). There are no AST nodes, no scope-tree queries, and no tree-sitter captures. All rules below describe line-pattern matching, not node traversal.

The scanner operates on trimmed lines, converting each to uppercase for keyword matching while recovering original-case names for symbol storage.

## Scanner Behaviour
- All keyword matching is on the uppercased line (`line.to_uppercase()`).
- Empty lines and lines starting with `'` (single-quote comment) or `REM ` are skipped entirely.
- Line continuation (trailing ` _`) is **not** spliced — continuation lines are processed independently. The underscore token is effectively ignored because it does not match any extraction pattern.
- `End Sub`, `End Function`, `End Property` (exact match or with trailing space) close the current procedure scope (`in_proc = false`). `current_proc` is not cleared on close; it stays set so trailing references before the next procedure header are attributed correctly.

## Symbol Extraction

| Logical Node | SymbolKind | Detection Pattern | Visibility | Notes |
|---|---|---|---|---|
| `sub_declaration` | `Function` | Line matches `[PUBLIC\|PRIVATE\|FRIEND\|STATIC\|PROTECTED ]SUB <name>` | `Public` (always) | Both `Sub` and `Function` map to `SymbolKind::Function` |
| `function_declaration` | `Function` | Line matches `[PUBLIC\|PRIVATE\|FRIEND\|STATIC\|PROTECTED ]FUNCTION <name>` | `Public` (always) | Return type annotation (`As Type`) is part of the stored signature but not separately parsed |
| `property_declaration` | `Property` | Line matches `[visibility ]PROPERTY GET\|LET\|SET <name>` | `Public` (always) | All three accessors (Get/Let/Set) emit the same `SymbolKind::Property` with the same name |
| `class_module` | `Class` | Line matches `ATTRIBUTE VB_NAME = "<name>"` | `Public` (always) | Only the first occurrence is used. The class symbol becomes the implicit parent for all subsequent Sub/Function/Property symbols in the file |
| `variable_declaration` | `Variable` | Line matches `DIM <name>` or `PUBLIC <name>` (without SUB/FUNCTION) or `PRIVATE <name>` (without SUB/FUNCTION) | `Public` (always) | Only emitted when **outside** a procedure body (`!in_proc`). Module-scope variables only; local variables inside Subs are not extracted |

### Visibility Note
`make_symbol` hardcodes `visibility: Some(Visibility::Public)` for all symbols regardless of the actual `Public`/`Private`/`Friend` keyword on the declaration line. The extractor does not differentiate visibility.

### Signature Storage
Procedure and property symbols store the full original-case declaration line as their `signature`. Variable symbols also store the full declaration line as their signature. Class symbols have no signature.

### Parent Relationships
When a `Attribute VB_Name` class symbol exists in the file, all Sub/Function/Property symbols set `parent_index` to the index of that class symbol. Variable declarations do not get a parent index even in class modules.

## Edge Extraction

### Calls
| Detection Pattern | Target Extraction | Condition | Notes |
|---|---|---|---|
| Line starts with `CALL ` | Token after `CALL ` up to first space or `(` | Only inside a procedure body (`in_proc`) | Explicit call syntax |
| First token is an identifier (not a keyword, no `.`, no `=`, no `(`) AND line contains a space or `(` AND line does not contain ` = ` | First token (up to `(`) | Only inside a procedure body (`in_proc`) | Implicit call heuristic — procedure name followed by arguments |

The implicit call heuristic filters out:
- Lines where the first token contains `.` (method calls on objects — not extracted)
- Lines where the first token contains `=` (assignments)
- Lines where the first token contains `(` (function-result-on-LHS)
- Lines that contain ` = ` anywhere (assignments)
- Lines where the first token matches the `is_vba_keyword` list (see below)
- Lines with no space and no `(` after the first token (bare labels or single-token statements)

All calls use `EdgeKind::Calls`. `module` and `chain` fields are `None`.

### Inherits / Implements
Not extracted. VBA `Implements InterfaceName` statements are not detected by the scanner.

### Imports
Not extracted. VBA `#include`-equivalent patterns (`CreateObject`, workbook references) are not detected as import edges.

## Type Annotation Locations
VBA uses `As <TypeName>` syntax for parameter and variable types. The extractor **does not parse** type annotations — they appear in stored signatures but are not emitted as `TypeRef` edges or structured fields.

Examples that appear in signatures but are not broken out:
```vba
Function Square(x As Integer) As Integer
Dim conn As ADODB.Connection
Public ws As Worksheet
```

## Doc Comment Convention
VBA comments use `'` (single-quote). The scanner **skips** all comment lines entirely — doc comments are not attached to symbols. `doc_comment` is always `None` on every emitted symbol.

`REM <text>` (legacy BASIC style) is also skipped.

## Test Detection
No test detection. VBA has no standard test framework with discoverable naming conventions that the scanner checks. `SymbolKind::Test` is never emitted.

## VBA Keyword Filter (Implicit Call Guard)
The following uppercase tokens are recognised as keywords and suppress implicit call emission:

`DIM`, `SET`, `LET`, `IF`, `ELSE`, `ELSEIF`, `END`, `FOR`, `NEXT`, `DO`, `LOOP`, `WHILE`, `WEND`, `SELECT`, `CASE`, `WITH`, `EXIT`, `GOTO`, `RESUME`, `ON`, `ERROR`, `RETURN`, `REM`, `OPTION`, `EXPLICIT`, `BASE`, `COMPARE`, `ME`, `NEW`, `NOT`, `AND`, `OR`, `XOR`, `IS`, `LIKE`, `MOD`, `NOTHING`, `EMPTY`, `NULL`, `TRUE`, `FALSE`, `MSGBOX`, `INPUTBOX`, `PRINT`, `DEBUG`, `OPEN`, `CLOSE`, `GET`, `PUT`, `SEEK`, `WRITE`, `INPUT`, `LINE`, `REDIM`, `ERASE`, `STOP`

## Builtin Symbols (`is_vba_builtin`)
These names are always in scope and should be filtered from unresolved-reference lists:

**UI**: `MsgBox`, `InputBox`, `Debug.Print`, `Debug.Assert`

**String**: `Len`, `Mid`, `Left`, `Right`, `Trim`, `LTrim`, `RTrim`, `UCase`, `LCase`, `InStr`, `InStrRev`, `Replace`, `Split`, `Join`

**Type conversion**: `Val`, `Str`, `CStr`, `CInt`, `CLng`, `CDbl`, `CSng`, `CBool`, `CDate`, `CByte`, `CVar`, `Format`

**Date/time**: `Now`, `Date`, `Time`, `Year`, `Month`, `Day`, `Hour`, `Minute`, `Second`, `DateAdd`, `DateDiff`, `DateSerial`, `TimeSerial`, `Timer`

**Type checks**: `IsNull`, `IsEmpty`, `IsNumeric`, `IsDate`, `IsArray`, `IsObject`, `IsMissing`, `IsError`, `TypeName`, `VarType`

**Array**: `Array`, `UBound`, `LBound`, `Erase`, `ReDim`

**File system**: `Dir`, `Kill`, `FileCopy`, `MkDir`, `RmDir`, `ChDir`, `ChDrive`, `CurDir`, `FileLen`, `FileDateTime`, `FreeFile`, `Open`, `Close`, `Input`, `Print`, `Write`, `Get`, `Put`, `Seek`, `EOF`, `LOF`

**System**: `Shell`, `Environ`, `CreateObject`, `GetObject`, `Err`, `Error`, `On`, `Resume`, `GoTo`, `Exit`, `End`, `Stop`, `DoEvents`

**Constants**: `Nothing`, `True`, `False`, `Null`, `Empty`, `vbCrLf`, `vbTab`, `vbNewLine`, `vbNullString`

## Edge Kind Compatibility
From `builtins::kind_compatible`:

| EdgeKind | Compatible symbol kinds |
|---|---|
| `Calls` | `method`, `function`, `constructor`, `test`, `class` |
| `Inherits` | `class` |
| `Implements` | `class`, `interface` |
| `TypeRef` | `class`, `interface`, `enum`, `type_alias`, `function`, `variable` |
| `Instantiates` | `class`, `function` |
| all others | always compatible |

## Unhandled Constructs
| Construct | Reason |
|---|---|
| `Implements InterfaceName` | Not detected — no `Implements` edge emitted |
| `Type ... End Type` (UDT) | Not detected — no `Struct`/`TypeAlias` symbol emitted |
| `Enum ... End Enum` | Not detected — no `Enum` symbol emitted |
| `Const <name> = <value>` | Not detected — no `Constant` symbol emitted |
| `Declare Function/Sub` (API declarations) | Not detected |
| `#If / #Else / #End If` (conditional compilation) | Not detected |
| `Option Explicit` / `Option Base` / `Option Compare` | Skipped (keyword filter) |
| Local `Dim` inside procedures | Skipped (`!in_proc` guard — only module-scope variables extracted) |
| Method calls on objects (`obj.Method args`) | Skipped (first token contains `.`) |
| `' doc comment` lines | Skipped entirely — never attached to symbols |
| `Class_Initialize` / `Class_Terminate` | Extracted as ordinary `Function` symbols (not a special constructor kind) |
| `WithEvents` variable declarations | Extracted as `Variable` if at module scope (keyword `WithEvents` follows `Dim`/`Public`, so name extraction picks up `WithEvents` — likely incorrect in edge cases) |
| Multi-variable `Dim a, b, c As Type` | Only first name extracted (split on space/comma/tab stops at first token) |
