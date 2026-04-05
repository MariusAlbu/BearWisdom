#!/usr/bin/env python3
"""
Transform tree-sitter generated parser.c to compile under MSVC (C mode).

Problem: MSVC C11 does not support C99 designated array initializers:
  [N] = value    (array element at index N)
  [ENUM] = value (array element at enum index)

This script rewrites all such patterns to be compatible with MSVC by
converting static array initializations to zero-fill + runtime assignment.

Approach for each array with designated initializers:
1. Declare the array as non-const with zero-initialization
2. Generate an init function that does the assignments
3. Wrap ts_parser_init() or similar to call init first

SIMPLER APPROACH: For the specific patterns in tree-sitter generated code,
we can just convert the C file to avoid designated array initializers by:
- Expanding sparse arrays to fully-specified dense arrays
  (filling gaps with 0 / NULL / appropriate zero value)

This works because:
- The arrays are small (PRODUCTION_ID_COUNT=27, LARGE_STATE_COUNT=15)
- The enum values used as indices are compile-time constants

Usage: python transform_parser.py <input.c> <output.c>
"""

import re
import sys


def get_enum_values(content):
    """Extract enum value → integer mappings from the C file."""
    enum_map = {}

    # Find all enum definitions
    for enum_match in re.finditer(r'enum\s*\{([^}]+)\}', content, re.DOTALL):
        body = enum_match.group(1)
        current_val = 0
        for item in re.split(r',', body):
            item = item.strip()
            if not item:
                continue
            if '=' in item:
                name, val_str = item.split('=', 1)
                name = name.strip()
                val_str = val_str.strip()
                try:
                    current_val = int(val_str, 0)
                except ValueError:
                    pass
                enum_map[name] = current_val
            else:
                name = item.strip()
                if name:
                    enum_map[name] = current_val
            current_val += 1

    return enum_map


def get_define_values(content):
    """Extract #define NAME value mappings."""
    define_map = {}
    for m in re.finditer(r'#define\s+(\w+)\s+(\d+)', content):
        define_map[m.group(1)] = int(m.group(2))
    return define_map


def resolve_index(idx_str, enum_map, define_map):
    """Resolve an index expression to an integer, or return None."""
    idx_str = idx_str.strip()
    try:
        return int(idx_str, 0)
    except ValueError:
        pass
    if idx_str in define_map:
        return define_map[idx_str]
    if idx_str in enum_map:
        return enum_map[idx_str]
    return None


def expand_1d_array(array_text, enum_map, define_map, array_size):
    """
    Expand a 1D array with designated initializers to a dense array.

    Input: '{ [0] = NULL, [field_name] = "name", ... }'
    Output: '{ NULL, NULL, ..., "name", ..., NULL }'
    """
    # Parse designated entries
    entries = {}
    # Pattern: [index_expr] = value (where value may be complex)
    # We'll tokenize the content between { and }
    content = array_text.strip()
    if content.startswith('{'):
        content = content[1:]
    if content.endswith('}'):
        content = content[:-1]

    # Split on commas that are not inside braces/parens
    items = split_at_top_level_commas(content)

    for item in items:
        item = item.strip()
        if not item:
            continue
        m = re.match(r'^\s*\[([^\]]+)\]\s*=\s*(.*)', item, re.DOTALL)
        if m:
            idx_str = m.group(1).strip()
            val = m.group(2).strip()
            idx = resolve_index(idx_str, enum_map, define_map)
            if idx is not None:
                entries[idx] = val

    if not entries:
        return array_text  # No change needed

    max_idx = max(entries.keys()) if entries else 0
    size = max(array_size if array_size else 0, max_idx + 1)

    # Build dense array
    result_parts = []
    for i in range(size):
        if i in entries:
            result_parts.append(entries[i])
        else:
            result_parts.append('0')

    return '{\n  ' + ',\n  '.join(result_parts) + '\n}'


def split_at_top_level_commas(s):
    """Split string at commas that are not inside braces, brackets, or parens."""
    parts = []
    depth = 0
    current = []
    for ch in s:
        if ch in '({[':
            depth += 1
            current.append(ch)
        elif ch in ')}]':
            depth -= 1
            current.append(ch)
        elif ch == ',' and depth == 0:
            parts.append(''.join(current))
            current = []
        else:
            current.append(ch)
    if current:
        parts.append(''.join(current))
    return parts


def transform_file(input_path, output_path):
    with open(input_path, 'r', encoding='utf-8') as f:
        content = f.read()

    enum_map = get_enum_values(content)
    define_map = get_define_values(content)

    print(f"Found {len(enum_map)} enum values, {len(define_map)} defines", file=sys.stderr)

    # Add field_* enum values by scanning field enum
    for m in re.finditer(r'field_(\w+)\s*=\s*(\d+)', content):
        enum_map[f'field_{m.group(1)}'] = int(m.group(2))

    # Add alias_sym_* enum values
    for m in re.finditer(r'(alias_sym_\w+)\s*=\s*(\d+)', content):
        enum_map[m.group(1)] = int(m.group(2))

    print(f"Total known symbols: {len(enum_map)}", file=sys.stderr)

    # Strategy: Find all static array declarations with designated initializers
    # and expand them. We process the whole content as text.

    # We need to handle the specific patterns in tree-sitter generated files.
    # The main problem arrays use integer or enum values as indices.

    # Pattern to match: static [qualifiers] TYPE array[SIZE] = { [N] = ... };
    # We'll do a multi-pass transformation

    transformed = transform_designated_initializers(content, enum_map, define_map)

    with open(output_path, 'w', encoding='utf-8') as f:
        f.write(transformed)

    print(f"Transformation complete: {output_path}", file=sys.stderr)


def transform_designated_initializers(content, enum_map, define_map):
    """
    Find and transform all designated array initializer blocks.

    Handles:
    1. static const TSFieldMapSlice arr[N] = { [k] = {.x = v} ... };
    2. static const char* arr[] = { [0] = NULL, [field_x] = "name" ... };
    3. Nested: arr[N][M] = { [outer] = { [inner] = val } }
    """

    # We use a state machine approach to find array initializer blocks
    # Look for designated initializers: [<expr>] =
    # and check if they're inside a static array declaration

    lines = content.split('\n')
    result_lines = []
    i = 0

    while i < len(lines):
        line = lines[i]

        # Check if this line starts a static array with designated initializers ahead
        # Look for: static [const] TYPE name[...] = {
        arr_decl = re.match(
            r'^(static\s+(?:const\s+)?(?:unsigned\s+)?(?:uint\w+_t|int|char|TSSymbol|TSFieldId|'
            r'TSFieldMapSlice|TSFieldMapEntry|TSStateId|bool|TSLexMode|TSParseActionEntry|'
            r'TSSymbolMetadata|TSValidSymbols|uint8_t|uint16_t|uint32_t)\s*'
            r'(?:\*\s*const\s*|\*\s*)?'
            r'(\w+)\s*\[(?:[^\]]*)\](?:\s*\[[^\]]*\])?\s*=\s*\{)\s*$',
            line
        )

        if arr_decl:
            # Collect the full array body
            block_lines = [line]
            brace_depth = line.count('{') - line.count('}')
            j = i + 1
            while j < len(lines) and brace_depth > 0:
                block_lines.append(lines[j])
                brace_depth += lines[j].count('{') - lines[j].count('}')
                j += 1

            block_text = '\n'.join(block_lines)

            # Check if this block has designated array initializers
            if re.search(r'\[\s*(?:\d+|field_\w+|alias_sym_\w+|sym_\w+|ts_builtin\w+|anon_sym\w+)\s*\]\s*=', block_text):
                # This block has designated initializers - we need to transform it
                transformed_block = transform_array_block(block_text, block_lines, enum_map, define_map)
                result_lines.extend(transformed_block.split('\n'))
                i = j
                continue

        result_lines.append(line)
        i += 1

    return '\n'.join(result_lines)


def transform_array_block(block_text, block_lines, enum_map, define_map):
    """Transform a single static array declaration block to remove designated initializers."""
    # For now, just strip the designated index markers
    # [N] = value  →  value  (for the common case where we can reorder)
    # This ONLY works if the indices are contiguous starting from 0
    # For sparse arrays, we need to expand them

    # Detect the declaration line to get array name and type
    first_line = block_lines[0]

    # Simple approach: remove [N] = markers where N is a simple integer
    # and restructure the entries in order

    # Extract the body (between first { and matching })
    brace_start = block_text.find('{')
    # Find matching close brace
    depth = 0
    brace_end = -1
    for k, ch in enumerate(block_text[brace_start:], brace_start):
        if ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
            if depth == 0:
                brace_end = k
                break

    if brace_end == -1:
        return block_text

    header = block_text[:brace_start]
    body = block_text[brace_start+1:brace_end]
    footer = block_text[brace_end+1:]

    # Parse the body entries
    entries = {}
    has_designated = False

    # Split body at top-level commas
    items = split_at_top_level_commas(body)

    for item in items:
        item = item.strip()
        if not item:
            continue
        m = re.match(r'^\[([^\]]+)\]\s*=\s*(.*)', item.strip(), re.DOTALL)
        if m:
            has_designated = True
            idx_str = m.group(1).strip()
            val = m.group(2).strip()
            idx = resolve_index(idx_str, enum_map, define_map)
            if idx is not None:
                entries[idx] = val
            else:
                # Can't resolve - keep as-is (will cause MSVC error but at least we tried)
                return block_text
        else:
            # Non-designated - keep position
            entries[len(entries)] = item

    if not has_designated:
        return block_text

    if not entries:
        return block_text

    max_idx = max(entries.keys())

    # Build dense result
    dense = []
    for k in range(max_idx + 1):
        if k in entries:
            val = entries[k]
            # Handle nested designated struct initializers: {.field = val}
            # Convert to positional: {val1, val2}
            val = convert_struct_designated(val, enum_map, define_map)
            dense.append(val)
        else:
            dense.append('{0}' if '{' in (entries.get(0, '') or '') else '0')

    new_body = ',\n  '.join(dense)
    return f"{header}{{{new_body}{footer}"


def convert_struct_designated(val, enum_map, define_map):
    """
    Convert {.field = v1, .field2 = v2} to {v1, v2} for TSFieldMapSlice-like structs.
    Also handles nested { [N] = inner_val } patterns.
    """
    val = val.strip()
    if not val.startswith('{'):
        return val

    inner = val[1:-1].strip() if val.endswith('}') else val[1:].strip()

    # Check if it has .field = designators
    if re.search(r'\.\w+\s*=', inner):
        # Extract positional values (assumes order matches struct definition)
        parts = split_at_top_level_commas(inner)
        positional = []
        for part in parts:
            part = part.strip()
            m = re.match(r'^\.\w+\s*=\s*(.*)', part, re.DOTALL)
            if m:
                positional.append(m.group(1).strip())
            else:
                positional.append(part)
        return '{' + ', '.join(positional) + '}'

    # Check if it has [N] = designators (nested array)
    if re.search(r'\[\s*(?:\d+|\w+)\s*\]\s*=', inner):
        parts = split_at_top_level_commas(inner)
        entries = {}
        for part in parts:
            part = part.strip()
            m = re.match(r'^\[([^\]]+)\]\s*=\s*(.*)', part, re.DOTALL)
            if m:
                idx_str = m.group(1).strip()
                v = m.group(2).strip()
                idx = resolve_index(idx_str, enum_map, define_map)
                if idx is not None:
                    entries[idx] = v

        if entries:
            max_idx = max(entries.keys())
            dense = []
            for k in range(max_idx + 1):
                dense.append(entries.get(k, '0'))
            return '{' + ', '.join(dense) + '}'

    return val


if __name__ == '__main__':
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} input.c output.c", file=sys.stderr)
        sys.exit(1)
    transform_file(sys.argv[1], sys.argv[2])
