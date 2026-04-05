#!/usr/bin/env python3
"""
Expand tree-sitter generated C arrays with designated initializers to be MSVC-compatible.

MSVC (even C11 mode) does not support C99 designated array initializers:
  arr[SIZE] = { [N] = value, ... }

This script converts them to zero-filled arrays with the entries in order:
  arr[SIZE] = { 0, ..., value_at_N, ..., 0 }

For 2D arrays, each row with designated initializers is similarly expanded.
"""

import re
import sys
import os


def collect_defines(content):
    """Collect all #define NAME INTEGER_VALUE mappings."""
    d = {}
    for m in re.finditer(r'^#define\s+(\w+)\s+([-]?\d+|0x[0-9a-fA-F]+)', content, re.MULTILINE):
        try:
            d[m.group(1)] = int(m.group(2), 0)
        except ValueError:
            pass
    return d


def collect_enum_values(content, defines):
    """Collect all enum NAME = integer_value mappings."""
    vals = {}

    for m in re.finditer(r'(?:typedef\s+)?enum\s*(?:\w+\s*)?\{([^}]+)\}', content, re.DOTALL):
        block = m.group(1)
        current = 0
        for item in re.split(r',', block):
            item = item.strip()
            if not item or item.startswith('//') or item.startswith('/*'):
                continue
            ma = re.match(r'(\w+)\s*=\s*([-]?(?:0x[0-9a-fA-F]+|\d+))', item)
            if ma:
                name = ma.group(1)
                try:
                    current = int(ma.group(2), 0)
                except ValueError:
                    pass
                vals[name] = current
            else:
                m2 = re.match(r'(\w+)', item)
                if m2:
                    vals[m2.group(1)] = current
            current += 1

    return vals


def resolve(expr, defines, enum_vals):
    """Resolve an expression to an integer index. Returns None if can't resolve."""
    expr = expr.strip()
    try:
        return int(expr, 0)
    except ValueError:
        pass
    if expr in defines:
        return defines[expr]
    if expr in enum_vals:
        return enum_vals[expr]
    # Handle SMALL_STATE(N) macro: SMALL_STATE(id) = id - LARGE_STATE_COUNT
    m = re.match(r'^SMALL_STATE\((\w+)\)$', expr)
    if m:
        inner = resolve(m.group(1), defines, enum_vals)
        large_count = defines.get('LARGE_STATE_COUNT', 0)
        if inner is not None:
            return inner - large_count
    # Try simple arithmetic: EXPR + N or EXPR - N
    for op in ['+', '-']:
        if op in expr:
            parts = expr.split(op, 1)
            left = resolve(parts[0].strip(), defines, enum_vals)
            right = resolve(parts[1].strip(), defines, enum_vals)
            if left is not None and right is not None:
                return left + right if op == '+' else left - right
    return None


def split_at_commas(s):
    """Split at top-level commas (not inside braces/parens/strings)."""
    parts = []
    depth = 0
    current = []
    i = 0
    in_string = False
    string_char = None
    while i < len(s):
        ch = s[i]
        if in_string:
            current.append(ch)
            if ch == string_char and (i == 0 or s[i-1] != '\\'):
                in_string = False
        elif ch in '"\'':
            in_string = True
            string_char = ch
            current.append(ch)
        elif ch in '({[':
            depth += 1
            current.append(ch)
        elif ch in ')}]':
            depth -= 1
            current.append(ch)
        elif ch == ',' and depth == 0:
            parts.append(''.join(current).strip())
            current = []
        elif ch == '/' and i + 1 < len(s) and s[i+1] == '*':
            # Block comment - consume until */
            end = s.find('*/', i + 2)
            if end >= 0:
                i = end + 2
                continue
        elif ch == '/' and i + 1 < len(s) and s[i+1] == '/':
            # Line comment - consume until newline
            end = s.find('\n', i)
            if end >= 0:
                i = end
                continue
        else:
            current.append(ch)
        i += 1
    if current:
        parts.append(''.join(current).strip())
    return [p for p in parts if p]


def expand_array_body(body, defines, enum_vals, array_size=None):
    """
    Expand array body from designated initializer form to dense form.

    Returns (expanded_body, was_modified).
    """
    items = split_at_commas(body)
    if not items:
        return body, False

    # Check if any item uses designated initializer syntax
    has_designated = any(re.match(r'^\s*\[', item) for item in items)
    if not has_designated:
        return body, False

    entries = {}
    pos = 0
    for item in items:
        item = item.strip()
        if not item:
            continue
        m = re.match(r'^\[([^\]]+)\]\s*=\s*(.*)', item, re.DOTALL)
        if m:
            idx_expr = m.group(1).strip()
            val = m.group(2).strip()
            idx = resolve(idx_expr, defines, enum_vals)
            if idx is None:
                # Can't resolve index - return original
                print(f"  WARNING: Cannot resolve index '{idx_expr}', skipping expansion", file=sys.stderr)
                return body, False
            entries[idx] = val
            pos = idx + 1
        else:
            # Non-designated entry - use current position
            entries[pos] = item
            pos += 1

    if not entries:
        return body, False

    max_idx = max(entries.keys())
    size = array_size if array_size and array_size > max_idx else max_idx + 1

    # Determine the default value for missing entries.
    # For 2D arrays (entries contain {…} values), use {0} instead of scalar 0.
    any_row_is_struct = any(
        v.strip().startswith('{') for v in entries.values()
    )
    default_val = '{0}' if any_row_is_struct else '0'

    # Build dense array
    dense = []
    for i in range(size):
        val = entries.get(i, default_val)
        # Recursively expand nested designated initializers in the value
        if '{' in val and '[' in val:
            val_inner = val[1:-1] if val.startswith('{') and val.endswith('}') else val
            expanded_inner, mod = expand_array_body(val_inner, defines, enum_vals)
            if mod:
                val = '{' + expanded_inner + '}'
        # Convert struct designated initializers: {.field = v} -> {v}
        if val.startswith('{') and re.search(r'\.\w+\s*=', val):
            val = convert_struct_designators(val)
        dense.append(val)

    return ',\n  '.join(dense), True


def convert_struct_designators(val):
    """
    Convert {.field = v1, .field2 = v2} to {v1, v2} recursively.
    Handles nested structs: {.a = {.x = 1, .y = 2}} -> {{1, 2}}.
    Assumes struct fields are in declaration order.
    """
    val = val.strip()
    if not val.startswith('{'):
        return val
    inner = val[1:-1] if val.endswith('}') else val[1:]
    inner = inner.strip()

    if not re.search(r'\.\w+\s*=', inner):
        # No struct designators at this level — but recurse into nested {}
        parts = split_at_commas(inner)
        converted = []
        changed = False
        for p in parts:
            p = p.strip()
            if p.startswith('{') and re.search(r'\.\w+\s*=', p):
                c = convert_struct_designators(p)
                converted.append(c)
                changed = True
            else:
                converted.append(p)
        if changed:
            return '{' + ', '.join(converted) + '}'
        return val

    parts = split_at_commas(inner)
    positional = []
    for p in parts:
        p = p.strip()
        m = re.match(r'^\.\w+\s*=\s*(.*)', p, re.DOTALL)
        if m:
            v = m.group(1).strip()
            # Recursively convert nested struct designators in the value
            if v.startswith('{'):
                v = convert_struct_designators(v)
            positional.append(v)
        else:
            # No designator — recursively convert if it's a struct literal
            if p.startswith('{'):
                p = convert_struct_designators(p)
            positional.append(p)

    return '{' + ', '.join(positional) + '}'


def get_array_size(decl_line, defines, enum_vals):
    """Extract the declared array size from declaration line, if available."""
    # Look for [SIZE] or [MACRO]
    m = re.search(r'\[(\w+)\]', decl_line)
    if m:
        size_str = m.group(1)
        return resolve(size_str, defines, enum_vals)
    return None


def count_braces_in_line(line):
    """Count net open braces in a line, ignoring strings and comments."""
    depth = 0
    in_string = False
    string_char = None
    i = 0
    while i < len(line):
        ch = line[i]
        if in_string:
            if ch == string_char and (i == 0 or line[i-1] != '\\'):
                in_string = False
        elif ch in '"\'':
            in_string = True
            string_char = ch
        elif ch == '/' and i + 1 < len(line) and line[i+1] == '/':
            break  # Line comment
        elif ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
        i += 1
    return depth


def rewrite_ts_language_struct(content):
    """
    Rewrite the TSLanguage struct initializer from designated-field form to
    positional form that MSVC C11 accepts.

    The parser.c generated by tree-sitter uses field-designator syntax:
      static const TSLanguage language = {
        .version = LANGUAGE_VERSION,
        .symbol_count = SYMBOL_COUNT,
        ...
      };
    MSVC does not support C99 designated struct initializers.

    We extract each named field value and emit the struct using positional
    initialization in the exact order defined by parser.h struct TSLanguage.
    """
    # Find the TSLanguage struct initializer
    pattern = r'(static const TSLanguage language\s*=\s*)\{([^}]*(?:\{[^}]*\}[^}]*)*)\};'
    m = re.search(r'static const TSLanguage language\s*=\s*\{', content)
    if not m:
        print("  No TSLanguage struct found, skipping rewrite", file=sys.stderr)
        return content

    start = m.start()
    open_brace = content.find('{', m.start())
    if open_brace < 0:
        return content
    close_brace = find_matching_close(content, open_brace)
    if close_brace < 0:
        return content

    header = content[start:open_brace]
    body = content[open_brace+1:close_brace]
    after = content[close_brace+1:]

    # Parse field = value pairs from the body
    fields = {}
    # Split at top-level commas
    parts = split_at_commas(body)
    for part in parts:
        part = part.strip()
        fm = re.match(r'^\s*\.([\w]+)\s*=\s*(.*)', part, re.DOTALL)
        if fm:
            key = fm.group(1)
            val = fm.group(2).strip()
            # Remove trailing comma if any
            if val.endswith(','):
                val = val[:-1].strip()
            fields[key] = val

    # Map parser.c field names to parser.h field names and order.
    # parser.c uses .version; parser.h has .abi_version
    if 'version' in fields and 'abi_version' not in fields:
        fields['abi_version'] = fields.pop('version')
    # parser.c uses TSLexMode for lex_modes; parser.h expects TSLexerMode*.
    # TSLexMode (2 fields) and TSLexerMode (3 fields) are layout-compatible for
    # the first two fields. Cast to suppress the type mismatch warning.
    if 'lex_modes' in fields:
        fields['lex_modes'] = f'(const TSLexerMode *){fields["lex_modes"]}'

    # TSLanguage field order as defined in parser.h
    field_order = [
        'abi_version', 'symbol_count', 'alias_count', 'token_count',
        'external_token_count', 'state_count', 'large_state_count',
        'production_id_count', 'field_count', 'max_alias_sequence_length',
        'parse_table', 'small_parse_table', 'small_parse_table_map',
        'parse_actions', 'symbol_names', 'field_names',
        'field_map_slices', 'field_map_entries', 'symbol_metadata',
        'public_symbol_map', 'alias_map', 'alias_sequences',
        'lex_modes', 'lex_fn', 'keyword_lex_fn', 'keyword_capture_token',
        'external_scanner',
        'primary_state_ids', 'name', 'reserved_words',
        'max_reserved_word_set_size', 'supertype_count', 'supertype_symbols',
        'supertype_map_slices', 'supertype_map_entries', 'metadata',
    ]

    # Emit only fields present in the source, in struct field order.
    # Gaps (fields present in parser.h but not in this parser.c) get emitted as 0.
    # We stop at the last field that IS present to avoid spurious trailing zeros.
    # Find the last field index in field_order that is present in the source.
    last_present = -1
    for idx, fname in enumerate(field_order):
        if fname in fields:
            last_present = idx

    positional = []
    for idx, fname in enumerate(field_order):
        if idx > last_present:
            break
        if fname in fields:
            positional.append(f'  /* .{fname} = */ {fields[fname]}')
        else:
            # Gap field — emit 0 (C will zero-init but MSVC needs explicit values
            # when there are trailing designated fields it might complain about)
            positional.append(f'  /* .{fname} = */ 0')

    new_body = ',\n'.join(positional)
    new_struct = header + '{\n' + new_body + '\n}'
    result = content[:start] + new_struct + after
    print(f"  Rewrote TSLanguage struct initializer ({len(fields)} fields)", file=sys.stderr)
    return result


def find_matching_close(text, open_pos):
    """Find the position of the matching } for the { at open_pos, string-aware."""
    depth = 0
    in_string = False
    string_char = None
    i = open_pos
    while i < len(text):
        ch = text[i]
        if in_string:
            if ch == string_char and (i == 0 or text[i-1] != '\\'):
                in_string = False
        elif ch in '"\'':
            in_string = True
            string_char = ch
        elif ch == '/' and i + 1 < len(text) and text[i+1] == '*':
            end = text.find('*/', i + 2)
            if end >= 0:
                i = end + 2
                continue
        elif ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def transform_file(input_path, output_path):
    with open(input_path, 'r', encoding='utf-8', errors='replace') as f:
        content = f.read()

    defines = collect_defines(content)
    # Add parser.h defines
    defines['ts_builtin_sym_end'] = 0
    defines['ts_builtin_sym_error'] = -1
    # SMALL_STATE(id) = id - LARGE_STATE_COUNT (handled in resolve())

    enum_vals = collect_enum_values(content, defines)
    # Merge defines into enum_vals for resolution
    all_vals = {**enum_vals, **defines}

    print(f"Symbols: {len(all_vals)} total", file=sys.stderr)

    output = []
    i = 0
    lines = content.split('\n')

    while i < len(lines):
        line = lines[i]

        # Check if this line starts a static array declaration ending with = {
        # followed by designated initializers
        if re.match(r'\s*static\s+', line) and line.rstrip().endswith('{') and '=' in line:
            # Check if next non-empty line has [N] = pattern
            j = i + 1
            while j < len(lines) and not lines[j].strip():
                j += 1
            if j < len(lines) and re.match(r'\s*\[', lines[j]):
                # Collect the whole array using string-aware brace counting
                array_lines = [line]
                brace_depth = count_braces_in_line(line)
                k = i + 1
                while k < len(lines) and brace_depth > 0:
                    array_lines.append(lines[k])
                    brace_depth += count_braces_in_line(lines[k])
                    k += 1

                array_text = '\n'.join(array_lines)
                # Find the body (between outer { and }) using string-aware matching
                open_pos = array_text.find('{')
                if open_pos >= 0:
                    close_pos = find_matching_close(array_text, open_pos)

                    if close_pos > open_pos:
                        header = array_text[:open_pos]
                        body = array_text[open_pos+1:close_pos]
                        # Include the closing '}' and everything after it
                        footer = array_text[close_pos:]

                        # Get declared size from header
                        array_size = get_array_size(line, all_vals, all_vals)

                        expanded, modified = expand_array_body(body, all_vals, all_vals, array_size)
                        if modified:
                            new_text = header + '{\n  ' + expanded + '\n' + footer
                            output.extend(new_text.split('\n'))
                            i = k
                            print(f"  Expanded array at line {i+1}", file=sys.stderr)
                            continue
                        else:
                            print(f"  NOT expanded array at line {i+1}: {line[:60]}", file=sys.stderr)

        output.append(line)
        i += 1

    result = '\n'.join(output)

    # Fix type name mismatches between grammar-generated code and current tree-sitter headers.
    # The grammar was generated with an older tree-sitter API where:
    #   TSFieldMapSlice was the name (parser.h now uses TSMapSlice)
    # We inject a typedef alias right after the first #include.
    # NOTE: TSLexMode is already defined in parser.h (different struct from TSLexerMode),
    # so we cannot redefine it. The TSLanguage.lex_modes field accepts TSLexerMode*
    # but the grammar uses TSLexMode; we handle this with an explicit cast in the struct.
    compat = (
        '\n/* MSVC compatibility: alias TSFieldMapSlice to current TSMapSlice */\n'
        '#ifndef TSFieldMapSlice\n'
        'typedef TSMapSlice TSFieldMapSlice;\n'
        '#endif\n\n'
    )
    # Insert after the first #include line
    first_include = result.find('#include')
    if first_include >= 0:
        nl = result.find('\n', first_include)
        if nl >= 0:
            result = result[:nl+1] + compat + result[nl+1:]
    print(f"  Injected type compatibility shim", file=sys.stderr)

    # Rewrite REDUCE() macro calls that use C99 named/designated macro arguments.
    # MSVC does not support named macro parameters; rewrite to positional form.
    # Patterns:
    #   REDUCE(.symbol = X, .child_count = N)            -> REDUCE(X, N, 0, 0)
    #   REDUCE(.symbol = X, .child_count = N, .dynamic_precedence = P, .production_id = Q)
    #                                                    -> REDUCE(X, N, P, Q)
    # The REDUCE macro signature is: (symbol_name, children, precedence, prod_id)
    def rewrite_reduce(m):
        args_str = m.group(1)
        sym = ''
        children = '0'
        dyn_prec = '0'
        prod_id = '0'
        for part in re.split(r',\s*', args_str):
            part = part.strip()
            kv = re.match(r'^\.([\w]+)\s*=\s*(.*)', part)
            if kv:
                key = kv.group(1)
                val = kv.group(2).strip()
                if key == 'symbol':
                    sym = val
                elif key == 'child_count':
                    children = val
                elif key == 'dynamic_precedence':
                    dyn_prec = val
                elif key == 'production_id':
                    prod_id = val
        if sym:
            return f'REDUCE({sym}, {children}, {dyn_prec}, {prod_id})'
        return m.group(0)  # leave unchanged if not parseable

    result = re.sub(r'\bREDUCE\(([^)]+)\)', rewrite_reduce, result)
    reduce_remaining = len(re.findall(r'\bREDUCE\(\.', result))
    print(f"Remaining REDUCE named-args calls: {reduce_remaining}", file=sys.stderr)

    # Rewrite the TSLanguage struct initializer.
    # The parser.c uses .version but parser.h (0.25 ABI) uses .abi_version.
    # MSVC doesn't support struct designated initializers at all.
    # We rewrite to positional form based on the known TSLanguage field order.
    # Field order (from parser.h struct TSLanguage):
    #   abi_version, symbol_count, alias_count, token_count, external_token_count,
    #   state_count, large_state_count, production_id_count, field_count,
    #   max_alias_sequence_length, parse_table, small_parse_table,
    #   small_parse_table_map, parse_actions, symbol_names, field_names,
    #   field_map_slices, field_map_entries, symbol_metadata, public_symbol_map,
    #   alias_map, alias_sequences, lex_modes, lex_fn, keyword_lex_fn,
    #   keyword_capture_token, external_scanner { states, symbol_map, create,
    #   destroy, scan, serialize, deserialize },
    #   primary_state_ids, name, reserved_words, max_reserved_word_set_size,
    #   supertype_count, supertype_symbols, supertype_map_slices,
    #   supertype_map_entries, metadata
    # This is a one-shot regex substitution on the known pattern.
    result = rewrite_ts_language_struct(result)

    with open(output_path, 'w', encoding='utf-8') as f:
        f.write(result)

    # Count remaining designated initializers (inside array bodies, not array declarations).
    # A real designated init is a [N] = pattern on a line that is NOT a static/extern/const
    # array declaration (those legitimately have [SIZE] = { in the type).
    remaining = 0
    for line in result.split('\n'):
        stripped = line.lstrip()
        if stripped.startswith(('static ', 'extern ', 'const ')):
            continue
        if re.search(r'\[\s*(?:\d+|[A-Z_a-z]\w*)\s*\]\s*=', stripped):
            remaining += 1
    print(f"Remaining designated initializers: {remaining}", file=sys.stderr)

    return remaining


if __name__ == '__main__':
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} input.c output.c")
        sys.exit(1)
    remaining = transform_file(sys.argv[1], sys.argv[2])
    sys.exit(0 if remaining == 0 else 1)
