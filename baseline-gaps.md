# Engine vs heuristic gap (corpus-wide)

Captured from `baseline-all.json` at 2026-05-24T00:00:00Z. 257 project DBs read, 0 skipped (no `.bearwisdom/index.db`).

Engine = `LanguageEngineHooks::resolve_ref` + core type-checker (strategies `<lang>_*`, `engine_*`, `ts_*`, `csharp_*`, …). Heuristic = `heuristic.rs` tier-2 fallback (strategies `heuristic_import`, `heuristic_qualified_name`, …). Heuristic counts are the discovery gap the engine should close.

## Per-language summary

| language | engine | heuristic | unresolved | engine% | heur% | unres% | top heuristic strategy |
|---|---:|---:|---:|---:|---:|---:|---|
| csharp | 3,290,528 | 0 | 4 | 100.00% | 0.00% | 0.00% | — |
| typescript | 2,386,441 | 0 | 89,517 | 96.38% | 0.00% | 3.62% | — |
| c | 1,890,222 | 0 | 47,837 | 97.53% | 0.00% | 2.47% | — |
| cpp | 1,558,153 | 0 | 93,652 | 94.33% | 0.00% | 5.67% | — |
| javascript | 751,985 | 0 | 60,495 | 92.55% | 0.00% | 7.45% | — |
| rust | 687,578 | 0 | 29,694 | 95.86% | 0.00% | 4.14% | — |
| java | 682,909 | 0 | 29,007 | 95.93% | 0.00% | 4.07% | — |
| dart | 494,829 | 0 | 19,047 | 96.29% | 0.00% | 3.71% | — |
| pascal | 431,352 | 0 | 17,964 | 96.00% | 0.00% | 4.00% | — |
| kotlin | 416,622 | 0 | 15,758 | 96.36% | 0.00% | 3.64% | — |
| php | 376,825 | 0 | 6,121 | 98.40% | 0.00% | 1.60% | — |
| go | 267,475 | 0 | 20,539 | 92.87% | 0.00% | 7.13% | — |
| zig | 266,656 | 0 | 14,764 | 94.75% | 0.00% | 5.25% | — |
| erlang | 256,938 | 0 | 10,948 | 95.91% | 0.00% | 4.09% | — |
| nim | 206,996 | 0 | 13,796 | 93.75% | 0.00% | 6.25% | — |
| python | 190,076 | 0 | 9,898 | 95.05% | 0.00% | 4.95% | — |
| elixir | 185,475 | 0 | 10,408 | 94.69% | 0.00% | 5.31% | — |
| ruby | 163,961 | 0 | 272 | 99.83% | 0.00% | 0.17% | — |
| lua | 136,301 | 0 | 20,416 | 86.97% | 0.00% | 13.03% | — |
| scala | 144,991 | 0 | 7,620 | 95.01% | 0.00% | 4.99% | — |
| groovy | 115,144 | 0 | 25,133 | 82.08% | 0.00% | 17.92% | — |
| r | 104,184 | 0 | 33,249 | 75.81% | 0.00% | 24.19% | — |
| haskell | 125,519 | 0 | 5,289 | 95.96% | 0.00% | 4.04% | — |
| fsharp | 123,042 | 0 | 4,622 | 96.38% | 0.00% | 3.62% | — |
| swift | 113,276 | 0 | 7,557 | 93.75% | 0.00% | 6.25% | — |
| fortran | 109,152 | 0 | 6,735 | 94.19% | 0.00% | 5.81% | — |
| ocaml | 94,694 | 0 | 12,392 | 88.43% | 0.00% | 11.57% | — |
| clojure | 84,268 | 0 | 11,880 | 87.64% | 0.00% | 12.36% | — |
| odin | 62,857 | 0 | 16,192 | 79.52% | 0.00% | 20.48% | — |
| nix | 46,621 | 0 | 3,731 | 92.59% | 0.00% | 7.41% | — |
| markdown | 39,346 | 0 | 8,764 | 81.78% | 0.00% | 18.22% | — |
| robot | 30,421 | 0 | 5,386 | 84.96% | 0.00% | 15.04% | — |
| ada | 25,350 | 0 | 1,690 | 93.75% | 0.00% | 6.25% | — |
| bicep | 10,723 | 0 | 13,553 | 44.17% | 0.00% | 55.83% | — |
| vue | 20,149 | 0 | 3,869 | 83.89% | 0.00% | 16.11% | — |
| prolog | 13,421 | 0 | 4,692 | 74.10% | 0.00% | 25.90% | — |
| matlab | 10,089 | 0 | 6,210 | 61.90% | 0.00% | 38.10% | — |
| svelte | 224 | 0 | 16,040 | 1.38% | 0.00% | 98.62% | — |
| gdscript | 13,402 | 0 | 236 | 98.27% | 0.00% | 1.73% | — |
| cmake | 10,596 | 0 | 1,557 | 87.19% | 0.00% | 12.81% | — |
| jupyter | 9,435 | 0 | 2,421 | 79.58% | 0.00% | 20.42% | — |
| starlark | 10,843 | 0 | 792 | 93.19% | 0.00% | 6.81% | — |
| shell | 8,636 | 0 | 2,406 | 78.21% | 0.00% | 21.79% | — |
| powershell | 9,970 | 0 | 41 | 99.59% | 0.00% | 0.41% | — |
| mdx | 3,576 | 0 | 4,724 | 43.08% | 0.00% | 56.92% | — |
| jinja | 6,341 | 0 | 1,599 | 79.86% | 0.00% | 20.14% | — |
| sql | 6,380 | 0 | 444 | 93.49% | 0.00% | 6.51% | — |
| perl | 6,744 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| gleam | 5,179 | 0 | 1,195 | 81.25% | 0.00% | 18.75% | — |
| hcl | 5,811 | 0 | 10 | 99.83% | 0.00% | 0.17% | — |
| eex | 4,164 | 0 | 1,065 | 79.63% | 0.00% | 20.37% | — |
| razor | 4,913 | 0 | 15 | 99.70% | 0.00% | 0.30% | — |
| gsp | 805 | 0 | 4,105 | 16.40% | 0.00% | 83.60% | — |
| heex | 3,436 | 0 | 625 | 84.61% | 0.00% | 15.39% | — |
| prisma | 3,843 | 0 | 6 | 99.84% | 0.00% | 0.16% | — |
| angular_template | 2,812 | 0 | 627 | 81.77% | 0.00% | 18.23% | — |
| blade | 1,886 | 0 | 785 | 70.61% | 0.00% | 29.39% | — |
| scss | 2,218 | 0 | 423 | 83.98% | 0.00% | 16.02% | — |
| html | 1,701 | 0 | 886 | 65.75% | 0.00% | 34.25% | — |
| vbnet | 886 | 0 | 1,031 | 46.22% | 0.00% | 53.78% | — |
| proto | 1,616 | 0 | 287 | 84.92% | 0.00% | 15.08% | — |
| vba | 1,616 | 0 | 72 | 95.73% | 0.00% | 4.27% | — |
| erb | 965 | 0 | 4 | 99.59% | 0.00% | 0.41% | — |
| graphql | 617 | 0 | 34 | 94.78% | 0.00% | 5.22% | — |
| puppet | 647 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| astro | 0 | 0 | 480 | 0.00% | 0.00% | 100.00% | — |
| dockerfile | 394 | 0 | 5 | 98.75% | 0.00% | 1.25% | — |
| ejs | 270 | 0 | 23 | 92.15% | 0.00% | 7.85% | — |
| rmarkdown | 114 | 0 | 61 | 65.14% | 0.00% | 34.86% | — |
| make | 94 | 0 | 20 | 82.46% | 0.00% | 17.54% | — |
| handlebars | 91 | 0 | 11 | 89.22% | 0.00% | 10.78% | — |
| cobol | 92 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| haml | 69 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| yaml | 12 | 0 | 48 | 20.00% | 0.00% | 80.00% | — |
| templ | 35 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| julius | 0 | 0 | 16 | 0.00% | 0.00% | 100.00% | — |
| pug | 12 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| quarto | 12 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| nunjucks | 8 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| slim | 6 | 0 | 0 | 100.00% | 0.00% | 0.00% | — |
| twig | 0 | 0 | 5 | 0.00% | 0.00% | 100.00% | — |
| liquid | 0 | 0 | 3 | 0.00% | 0.00% | 100.00% | — |

## Per-language gap surface

For each language with non-trivial heuristic or unresolved volume, the top heuristic strategies (engine missed, fallback caught) and top unresolved targets (everything missed) follow. A heuristic strategy with a thousand+ hits is a candidate for an engine path.

### typescript
Engine 2,386,441 · Heuristic 0 · Unresolved 89,517 · (engine 96.38% of attempts)

Top unresolved targets:
- `expect` — 2,949
- `it` — 1,966
- `PrismaClient` — 1,524
- `Order` — 1,291
- `findUnique` — 938
- `FormlyFieldConfig` — 890
- `findMany` — 872
- `Types` — 810
- `Record` — 789
- `WithElementRef` — 780

### c
Engine 1,890,222 · Heuristic 0 · Unresolved 47,837 · (engine 97.53% of attempts)

Top unresolved targets:
- `CURLcode` — 1,416
- `CURL` — 1,367
- `lua_State` — 1,091
- `curl_easy_init` — 876
- `curl_easy_perform` — 784
- `curl_easy_cleanup` — 480
- `printf` — 474
- `p` — 380
- `ngx_sprintf` — 335
- `curl_easy_setopt` — 296

### cpp
Engine 1,558,153 · Heuristic 0 · Unresolved 93,652 · (engine 94.33% of attempts)

Top unresolved targets:
- `T1` — 1,692
- `lString16` — 1,354
- `OutputIt` — 1,003
- `IoExecutor` — 956
- `MutableBufferSequence` — 742
- `iterator_t` — 719
- `T2` — 704
- `ConstBufferSequence` — 632
- `charT` — 630
- `CComPtr` — 550

### javascript
Engine 751,985 · Heuristic 0 · Unresolved 60,495 · (engine 92.55% of attempts)

Top unresolved targets:
- `$` — 5,811
- `it` — 3,146
- `describe` — 1,416
- `on` — 813
- `findOne` — 689
- `dispatch` — 674
- `val` — 667
- `svg_jar` — 604
- `innerMode` — 545
- `CommonHelper` — 482

### rust
Engine 687,578 · Heuristic 0 · Unresolved 29,694 · (engine 95.86% of attempts)

Top unresolved targets:
- `map` — 713
- `input_value` — 510
- `to_string` — 475
- `cell` — 436
- `clone` — 433
- `Clone` — 376
- `assert` — 368
- `push` — 338
- `Number` — 301
- `PyResult` — 293

### java
Engine 682,909 · Heuristic 0 · Unresolved 29,007 · (engine 95.93% of attempts)

Top unresolved targets:
- `LOG` — 2,064
- `super` — 2,022
- `assertTrue` — 1,594
- `andReturn` — 1,210
- `assertFalse` — 873
- `isEqualTo` — 716
- `org` — 645
- `METHODS` — 634
- `this` — 444
- `setComponentTagClearTagState` — 419

### dart
Engine 494,829 · Heuristic 0 · Unresolved 19,047 · (engine 96.29% of attempts)

Top unresolved targets:
- `_i1` — 1,707
- `symmetric` — 1,062
- `json` — 1,048
- `i0` — 1,034
- `all` — 823
- `only` — 669
- `shrink` — 580
- `ViewPB` — 568
- `value` — 431
- `_` — 352

### pascal
Engine 431,352 · Heuristic 0 · Unresolved 17,964 · (engine 96.00% of attempts)

Top unresolved targets:
- `TVector3` — 3,372
- `TVector2` — 1,028
- `TX3DNode` — 760
- `isLB_AK` — 568
- `TAbstractTextureNode` — 376
- `PBIGNUM` — 279
- `isWB_AHLetter` — 200
- `TInt32List` — 192
- `DllImport` — 152
- `PPropInfo` — 130

### kotlin
Engine 416,622 · Heuristic 0 · Unresolved 15,758 · (engine 96.36% of attempts)

Top unresolved targets:
- `hasSize` — 1,284
- `assertTrue` — 745
- `InternalAPI` — 355
- `jsonPath` — 328
- `assertNotNull` — 231
- `LOGGER` — 229
- `isOk` — 205
- `assertFalse` — 191
- `CPointer` — 175
- `assertNull` — 165

### php
Engine 376,825 · Heuristic 0 · Unresolved 6,121 · (engine 98.40% of attempts)

Top unresolved targets:
- `assertEquals` — 1,154
- `fetch` — 668
- `isset` — 228
- `assign` — 214
- `str_replace` — 148
- `assertTrue` — 112
- `assertSeeIn` — 102
- `assertMissing` — 100
- `assertSame` — 98
- `file_exists` — 86

### go
Engine 267,475 · Heuristic 0 · Unresolved 20,539 · (engine 92.87% of attempts)

Top unresolved targets:
- `expected` — 2,477
- `genericParseType` — 649
- `expectedErrors` — 354
- `key` — 295
- `Request` — 280
- `Client` — 272
- `expectedConfig` — 251
- `email` — 227
- `value` — 224
- `record` — 223

### zig
Engine 266,656 · Heuristic 0 · Unresolved 14,764 · (engine 94.75% of attempts)

Top unresolved targets:
- `Register.Encoded` — 942
- `std.ArrayList` — 207
- `std.builtin.Signedness` — 168
- `maxInt` — 164
- `Node.Index` — 126
- `Register.GeneralSize` — 125
- `Node.OptionalIndex` — 124
- `Value.Index` — 122
- `Ref` — 118
- `err` — 103

### erlang
Engine 256,938 · Heuristic 0 · Unresolved 10,948 · (engine 95.91% of attempts)

Top unresolved targets:
- `'queue.declare'` — 500
- `Fun` — 409
- `'P_basic'` — 353
- `'queue.declare_ok'` — 334
- `'basic.publish'` — 263
- `oneof/1` — 221
- `'basic.get'` — 221
- `'basic.deliver'` — 201
- `'queue.bind'` — 191
- `gen_server` — 186

### nim
Engine 206,996 · Heuristic 0 · Unresolved 13,796 · (engine 93.75% of attempts)

Top unresolved targets:
- `Rune` — 1,484
- `nim` — 495
- `ColumnIndex` — 245
- `tryGet` — 215
- `cint` — 198
- `Color` — 169
- `chronicles` — 149
- `Epoch` — 148
- `ValidatorIndex` — 123
- `stew/byteutils` — 109

### python
Engine 190,076 · Heuristic 0 · Unresolved 9,898 · (engine 95.05% of attempts)

Top unresolved targets:
- `_` — 763
- `value_counts` — 564
- `Elements` — 407
- `execute_command` — 213
- `GetChildMemberWithName` — 193
- `Reviewer_Nationality` — 165
- `policy` — 115
- `grpc_channel` — 115
- `display` — 110
- `append` — 89

### elixir
Engine 185,475 · Heuristic 0 · Unresolved 10,408 · (engine 94.69% of attempts)

Top unresolved targets:
- `json_response` — 1,240
- `Repo` — 481
- `unquote` — 340
- `MixProject` — 260
- `CommandBehaviour` — 193
- `html_response` — 188
- `Teams` — 176
- `Billing` — 164
- `Goals` — 154
- `Email` — 127

### ruby
Engine 163,961 · Heuristic 0 · Unresolved 272 · (engine 99.83% of attempts)

Top unresolved targets:
- `ARGV` — 51
- `STDERR` — 37
- `ENV` — 20
- `puts` — 19
- `ALLOWED_OPTIONS` — 10
- `assert` — 10
- `Digest::SHA1` — 9
- `Gem::Version` — 8
- `Proc` — 7
- `ADDER_PACKAGE_NAME` — 6

### lua
Engine 136,301 · Heuristic 0 · Unresolved 20,416 · (engine 86.97% of attempts)

Top unresolved targets:
- `gsub` — 1,169
- `TEST` — 952
- `it` — 723
- `sub` — 614
- `same` — 573
- `equal` — 467
- `saveSetting` — 433
- `emit_signal` — 387
- `V` — 300
- `describe` — 289

### scala
Engine 144,991 · Heuristic 0 · Unresolved 7,620 · (engine 95.01% of attempts)

Top unresolved targets:
- `SuccessType` — 227
- `AsyncStream` — 118
- `forAll` — 113
- `MethodPerEndpoint` — 90
- `JHashMap` — 79
- `TwitterModule` — 75
- `CheckResult` — 68
- `ConstraintValidatorContext` — 66
- `EmbeddedTwitterServer` — 64
- `RouteIndex` — 64

### groovy
Engine 115,144 · Heuristic 0 · Unresolved 25,133 · (engine 82.08% of attempts)

Top unresolved targets:
- `column` — 2,872
- `changeSet` — 819
- `Mock` — 742
- `contains` — 514
- `addForeignKeyConstraint` — 324
- `get` — 307
- `add` — 306
- `http` — 285
- `resolve` — 250
- `mockDomain` — 233

### r
Engine 104,184 · Heuristic 0 · Unresolved 33,249 · (engine 75.81% of attempts)

Top unresolved targets:
- `slice_head` — 2,109
- `aes` — 1,767
- `ggplot` — 1,596
- `mutate` — 855
- `read_csv` — 855
- `theme` — 798
- `bind_cols` — 627
- `set_engine` — 627
- `set_mode` — 627
- `element_text` — 570

### haskell
Engine 125,519 · Heuristic 0 · Unresolved 5,289 · (engine 95.96% of attempts)

Top unresolved targets:
- `test'` — 208
- `functionResult` — 199
- `<#>` — 105
- `.=` — 100
- `since` — 91
- `findEntryByPath` — 85
- `-<` — 81
- `toEntry` — 73
- `###` — 72
- `=#>` — 68

### fsharp
Engine 123,042 · Heuristic 0 · Unresolved 4,622 · (engine 96.38% of attempts)

Top unresolved targets:
- `xs.Add` — 108
- `li.Add` — 93
- `throwsAnyError` — 86
- `source.Trigger` — 70
- `(=)` — 61
- `Seq.insertAt` — 54
- `Seq.insertManyAt` — 54
- `Seq.updateAt` — 54
- `Set.singleton` — 54
- `Seq.removeManyAt` — 51

### swift
Engine 113,276 · Heuristic 0 · Unresolved 7,557 · (engine 93.75% of attempts)

Top unresolved targets:
- `init` — 242
- `response` — 204
- `name` — 191
- `request` — 187
- `login` — 145
- `avatarUrl` — 140
- `$0` — 125
- `value` — 124
- `error` — 99
- `url` — 93

### fortran
Engine 109,152 · Heuristic 0 · Unresolved 6,735 · (engine 94.19% of attempts)

Top unresolved targets:
- `ptr` — 434
- `data` — 328
- `x1` — 180
- `xm1` — 180
- `dim_sizes` — 177
- `x2` — 168
- `xm2` — 168
- `ia` — 160
- `run_type` — 160
- `x3` — 159

### ocaml
Engine 94,694 · Heuristic 0 · Unresolved 12,392 · (engine 88.43% of attempts)

Top unresolved targets:
- `length` — 438
- `sprintf` — 368
- `field` — 323
- `fold_left` — 300
- `rev` — 239
- `iter` — 222
- `printf` — 210
- `field_o` — 199
- `returning` — 185
- `run` — 169

### clojure
Engine 84,268 · Heuristic 0 · Unresolved 11,880 · (engine 87.64% of attempts)

Top unresolved targets:
- `Exception` — 312
- `entry-function` — 260
- `v` — 157
- `clojure-core-ns` — 130
- `sci.core` — 129
- `tns` — 129
- `d` — 119
- `int?` — 117
- `nodes` — 101
- `*test-db` — 97

### odin
Engine 62,857 · Heuristic 0 · Unresolved 16,192 · (engine 79.52% of attempts)

Top unresolved targets:
- `msgSend` — 2,368
- `u1` — 2,193
- `GetDeviceProcAddr` — 1,204
- `GetInstanceProcAddr` — 706
- `(^uintptr)` — 333
- `write_string` — 305
- `Errno` — 276
- `(^u32x4)` — 130
- `ev` — 130
- `([^]byte)` — 123

### nix
Engine 46,621 · Heuristic 0 · Unresolved 3,731 · (engine 92.59% of attempts)

Top unresolved targets:
- `either` — 596
- `fetchurl` — 186
- `coercedTo` — 150
- `nixpkgs.lib.genAttrs` — 79
- `t.listOf` — 70
- `l.foldl'` — 61
- `nixpkgs.legacyPackages.${system}` — 54
- `t.nullOr` — 54
- `t.attrsOf` — 42
- `l.removePrefix` — 41

### markdown
Engine 39,346 · Heuristic 0 · Unresolved 8,764 · (engine 81.78% of attempts)

Top unresolved targets:
- `install.packages` — 561
- `../../..` — 540
- `enable` — 389
- `response` — 120
- `corr` — 114
- `operation` — 112
- `rename` — 112
- `../../../../4-Classification/data/ingredient_indexes` — 108
- `println` — 91
- `flyway` — 82

### robot
Engine 30,421 · Heuristic 0 · Unresolved 5,386 · (engine 84.96% of attempts)

Top unresolved targets:
- `Should Be Equal` — 291
- `New Page` — 273
- `Run Keyword And Expect Error` — 208
- `Log` — 156
- `New Context` — 136
- `Click` — 133
- `Set Browser Timeout` — 126
- `Get Title` — 108
- `Set Strict Mode` — 96
- `Check Keyword Data` — 89

### ada
Engine 25,350 · Heuristic 0 · Unresolved 1,690 · (engine 93.75% of attempts)

Top unresolved targets:
- `Send_Command` — 36
- `This.Port.Mem_Read` — 18
- `This.CS.Clear` — 15
- `This.CS.Set` — 15
- `This.Time.Delay_Milliseconds` — 15
- `Register` — 14
- `This.Receive` — 14
- `Display.Hidden_Buffer(1).Set_Source` — 13
- `This.Port.Mem_Write` — 13
- `At_Least_Within_Major` — 11

### bicep
Engine 10,723 · Heuristic 0 · Unresolved 13,553 · (engine 44.17% of attempts)

Top unresolved targets:
- `description` — 6,303
- `resourceGroup` — 1,049
- `resourceId` — 871
- `allowed` — 786
- `uniqueString` — 584
- `guid` — 323
- `subscription` — 294
- `range` — 288
- `secure` — 274
- `empty` — 265

### vue
Engine 20,149 · Heuristic 0 · Unresolved 3,869 · (engine 83.89% of attempts)

Top unresolved targets:
- `VCol` — 333
- `VIcon` — 315
- `VBtn` — 281
- `VRow` — 229
- `Variant` — 213
- `$emit` — 123
- `InertiaLink` — 114
- `VListItem` — 104
- `VListItemTitle` — 95
- `Story` — 94

### prolog
Engine 13,421 · Heuristic 0 · Unresolved 4,692 · (engine 74.10% of attempts)

Top unresolved targets:
- `tnot` — 3,007
- `get_residual` — 192
- `abolish_all_tables` — 98
- `'$fast_call'` — 65
- `'$module_call'` — 65
- `'$prepare_call_clause'` — 64
- `incr_assert` — 56
- `phrase` — 54
- `abolish_table_pred` — 49
- `must_be` — 46

### matlab
Engine 10,089 · Heuristic 0 · Unresolved 6,210 · (engine 61.90% of attempts)

Top unresolved targets:
- `pdist2` — 707
- `randperm` — 373
- `Data` — 235
- `SOLUTION` — 211
- `fliplr` — 174
- `bsxfun` — 133
- `unifrnd` — 121
- `ind` — 100
- `dbstack` — 96
- `uibutton` — 95

### svelte
Engine 224 · Heuristic 0 · Unresolved 16,040 · (engine 1.38% of attempts)

Top unresolved targets:
- `Field` — 1,547
- `DropdownMenu` — 1,150
- `Card` — 1,138
- `Button` — 828
- `Sidebar` — 761
- `IconPlaceholder` — 758
- `Item` — 748
- `InputGroup` — 528
- `Select` — 481
- `Example` — 389

### gdscript
Engine 13,402 · Heuristic 0 · Unresolved 236 · (engine 98.27% of attempts)

Top unresolved targets:
- `PoolStringArray` — 49
- `find_node` — 49
- `inst_to_dict` — 14
- `is_instance_of` — 14
- `PoolVector2Array` — 11
- `dict_to_inst` — 8
- `print_stack` — 8
- `cursor_get_column` — 8
- `print_debug` — 7
- `cursor_set_column` — 7

### cmake
Engine 10,596 · Heuristic 0 · Unresolved 1,557 · (engine 87.19% of attempts)

Top unresolved targets:
- `STAGING_DIR` — 60
- `MONOLIBTIC` — 35
- `lua54_SOURCE_DIR` — 32
- `uriparser_SOURCE_DIR` — 30
- `OpenAL_SOURCE_DIR` — 29
- `OUTPUT_DIR` — 21
- `${PROJECT_NAME}_core` — 20
- `po_build_file` — 19
- `OpenAL_BINARY_DIR` — 19
- `Coverage_NAME` — 18

### jupyter
Engine 9,435 · Heuristic 0 · Unresolved 2,421 · (engine 79.58% of attempts)

Top unresolved targets:
- `install.packages` — 504
- `pacman::p_load` — 504
- `suppressWarnings` — 504
- `rename` — 392
- `conf_mat` — 112
- `cor` — 112
- `corr` — 112
- `fviz_cluster` — 56
- `hist` — 56
- `write_csv` — 56

### starlark
Engine 10,843 · Heuristic 0 · Unresolved 792 · (engine 93.19% of attempts)

Top unresolved targets:
- `scala_artifact` — 78
- `scala_library` — 78
- `derive` — 34
- `is_normalized` — 27
- `DepsetBuilder` — 27
- `open_context` — 25
- `package_repo_name` — 23
- `get_platforms_os_name` — 22
- `get_npm_auth` — 19
- `split_extension` — 15

### shell
Engine 8,636 · Heuristic 0 · Unresolved 2,406 · (engine 78.21% of attempts)

Top unresolved targets:
- `runuser` — 190
- `oc` — 84
- `az` — 59
- `_filedir` — 47
- `dpkg` — 46
- `rlocation` — 46
- `chkconfig` — 41
- `rpm` — 41
- `fbink` — 41
- `mysql` — 36

### mdx
Engine 3,576 · Heuristic 0 · Unresolved 4,724 · (engine 43.08% of attempts)

Top unresolved targets:
- `LinkCard` — 749
- `TabItem` — 632
- `Tabs` — 261
- `Example` — 208
- `Card` — 201
- `Aside` — 188
- `FileTree` — 183
- `Steps` — 171
- `mavenCentral` — 126
- `AppOnly` — 124

### jinja
Engine 6,341 · Heuristic 0 · Unresolved 1,599 · (engine 79.86% of attempts)

Top unresolved targets:
- `k8s_image_pull_policy` — 51
- `external_hcloud_cloud` — 44
- `metallb_namespace` — 37
- `kube_config_dir` — 36
- `etcd_cert_dir` — 25
- `hostvars` — 24
- `matrix_synapse_worker_container_name` — 23
- `nodelocaldns_ip` — 21
- `node_pod_cidr` — 20
- `bin_dir` — 19

### sql
Engine 6,380 · Heuristic 0 · Unresolved 444 · (engine 93.49% of attempts)

Top unresolved targets:
- `public.post_aggregates` — 76
- `UInt64` — 63
- `votes` — 18
- `participants` — 17
- `comments` — 14
- `options` — 12
- `halfvec` — 10
- `sparsevec` — 10
- `series_metadata` — 10
- `UInt32` — 7

### gleam
Engine 5,179 · Heuristic 0 · Unresolved 1,195 · (engine 81.25% of attempts)

Top unresolved targets:
- `<>` — 94
- `+` — 69
- `field` — 64
- `success` — 51
- `set_body` — 51
- `==` — 50
- `-` — 38
- `continue` — 32
- `try` — 29
- `*` — 23

### eex
Engine 4,164 · Heuristic 0 · Unresolved 1,065 · (engine 79.63% of attempts)

Top unresolved targets:
- `AdminHelpers` — 329
- `Routes` — 241
- `SharedHelpers` — 160
- `PublicHelpers` — 40
- `SharedView` — 32
- `NewsItemView` — 26
- `TimeView` — 23
- `Endpoint` — 15
- `NewsItem` — 13
- `U` — 9

### gsp
Engine 805 · Heuristic 0 · Unresolved 4,105 · (engine 16.40% of attempts)

Top unresolved targets:
- `resource` — 1,294
- `hasErrors` — 511
- `message` — 487
- `fieldValue` — 301
- `$` — 260
- `createLink` — 114
- `metadata` — 107
- `getAttribute` — 68
- `ready` — 49
- `isCanceled` — 40

### heex
Engine 3,436 · Heuristic 0 · Unresolved 625 · (engine 84.61% of attempts)

Top unresolved targets:
- `AdminHelpers` — 156
- `Routes` — 138
- `SharedHelpers` — 85
- `PublicHelpers` — 27
- `TimeView` — 18
- `SharedView` — 11
- `Teams` — 11
- `Podcast` — 9
- `AuthView` — 9
- `Users` — 7

### angular_template
Engine 2,812 · Heuristic 0 · Unresolved 627 · (engine 81.77% of attempts)

Top unresolved targets:
- `mat-button` — 17
- `ng-option-tmp` — 14
- `#basicInfoForm` — 13
- `#abpBody` — 12
- `#abpFooter` — 12
- `#abpHeader` — 12
- `#inputField` — 12
- `#locationForm` — 12
- `#buttonOptions` — 11
- `ng-label-tmp` — 9

### blade
Engine 1,886 · Heuristic 0 · Unresolved 785 · (engine 70.61% of attempts)

Top unresolved targets:
- `__` — 41
- `renderHook` — 30
- `csrf_token` — 22
- `$getId` — 21
- `generate_icon_html` — 18
- `$applyStateBindingModifiers` — 15
- `$getExtraAttributes` — 15
- `$getChildSchema` — 14
- `$getFieldWrapperView` — 14
- `$getStatePath` — 13

### scss
Engine 2,218 · Heuristic 0 · Unresolved 423 · (engine 83.98% of attempts)

Top unresolved targets:
- `nb-install-component` — 116
- `nb-rtl` — 59
- `nb-ltr` — 44
- `media-breakpoint-down` — 37
- `it` — 32
- `assert` — 14
- `expect` — 14
- `output` — 14
- `nb-for-theme` — 12
- `assert-equal` — 11

### html
Engine 1,701 · Heuristic 0 · Unresolved 886 · (engine 65.75% of attempts)

Top unresolved targets:
- `Example` — 206
- `$` — 84
- `ScssDocs` — 66
- `Callout` — 61
- `BsTable` — 27
- `Placeholder` — 27
- `AppOnly` — 23
- `Livewire` — 21
- `PagesOnly` — 19
- `Check` — 18

### vbnet
Engine 886 · Heuristic 0 · Unresolved 1,031 · (engine 46.22% of attempts)

Top unresolved targets:
- `NameOf` — 64
- `CType` — 46
- `GetType` — 36
- `NotImplementedException` — 28
- `AddSingleton` — 19
- `OnPropertyChanged` — 19
- `TryCast` — 17
- `Where` — 17
- `CommunityToolkit.Mvvm.ComponentModel` — 14
- `List` — 13

### proto
Engine 1,616 · Heuristic 0 · Unresolved 287 · (engine 84.92% of attempts)

Top unresolved targets:
- `google.protobuf.Timestamp` — 49
- `google.protobuf.Duration` — 47
- `google.protobuf.StringValue` — 20
- `google.protobuf.Any` — 19
- `google.protobuf.Int32Value` — 18
- `google.protobuf.FloatValue` — 15
- `google.protobuf.BoolValue` — 14
- `google.protobuf.FieldMask` — 13
- `google.protobuf.UInt32Value` — 13
- `google.protobuf.DoubleValue` — 12

### vba
Engine 1,616 · Heuristic 0 · Unresolved 72 · (engine 95.73% of attempts)

Top unresolved targets:
- `Unload` — 9
- `CopyPicture` — 6
- `ExecuteMso` — 6
- `oRet` — 4
- `scopes` — 4
- `Create` — 3
- `ExecuteExcel4Macro` — 2
- `Found` — 2
- `LoadFile` — 2
- `MacScript` — 2

### astro
Engine 0 · Heuristic 0 · Unresolved 480 · (engine 0.00% of attempts)

Top unresolved targets:
- `Card` — 96
- `Placeholder` — 55
- `FontAwesome` — 24
- `Icon` — 22
- `Layout` — 18
- `Code` — 14
- `Main` — 12
- `LinkButton` — 10
- `Footer` — 9
- `Button` — 7

### rmarkdown
Engine 114 · Heuristic 0 · Unresolved 61 · (engine 65.14% of attempts)

Top unresolved targets:
- `knitr::opts_chunk.set` — 24
- `install.packages` — 9
- `pacman::p_load` — 9
- `suppressWarnings` — 7
- `conf_mat` — 2
- `cor` — 2
- `pak::pak` — 2
- `htmltools::tags.button` — 2
- `fviz_cluster` — 1
- `rmd2jupyter` — 1

## Per-project gap

Projects with the largest heuristic + unresolved combined volume.

| project | engine | heuristic | unresolved |
|---|---:|---:|---:|
| zig-compiler-fresh | 2,407,979 | 0 | 50,908 |
| lua-luals | 342,643 | 0 | 38,793 |
| jupyter-ml-for-beginners | 64,731 | 0 | 34,477 |
| ts-nextjs | 888,265 | 0 | 31,441 |
| lua-koreader | 211,049 | 0 | 28,135 |
| gsp-openboxes | 40,990 | 0 | 22,517 |
| pascal-castle-fresh | 321,141 | 0 | 19,809 |
| svelte-shadcn | 11,808 | 0 | 17,269 |
| odin-compiler | 157,206 | 0 | 16,290 |
| go-pocketbase | 69,627 | 0 | 14,692 |
| bicep-azure-quickstart-templates | 4,079 | 0 | 14,667 |
| javascript-ghost | 290,742 | 0 | 14,440 |
| dart-appflowy | 161,854 | 0 | 13,402 |
| gsp-grails-core | 183,248 | 0 | 13,366 |
| velocity-apache-struts | 189,386 | 0 | 11,384 |
| clojure-babashka | 71,440 | 0 | 10,321 |
| ocaml-dune-fresh | 66,930 | 0 | 10,190 |
| ts-immich | 315,472 | 0 | 10,122 |
| lua-awesome | 34,993 | 0 | 10,025 |
| erlang-rabbitmq | 144,487 | 0 | 9,511 |
| make-curl | 91,500 | 0 | 9,384 |
| elixir-plausible | 134,777 | 0 | 8,800 |
| kotlin-ktor | 194,373 | 0 | 8,644 |
| prisma-calcom | 296,144 | 0 | 8,234 |
| thymeleaf-myblog | 324,828 | 0 | 8,098 |
| nim-compiler | 112,782 | 0 | 7,898 |
| dotnet-abp | 472,509 | 0 | 7,814 |
| ts-ever-demand | 93,111 | 0 | 7,769 |
| fsharp-fable | 148,328 | 0 | 7,163 |
| prolog-swipl | 96,635 | 0 | 6,859 |
| dotnet-squidex | 254,437 | 0 | 6,605 |
| ruby-chatwoot | 75,922 | 0 | 6,570 |
| fortran-stdlib | 104,630 | 0 | 6,497 |
| erlang-emqx | 145,961 | 0 | 6,015 |
| dotnet-aspnetcore | 1,102,667 | 0 | 5,998 |
| groovy-nextflow | 55,286 | 0 | 5,516 |
| go-fiber | 91,886 | 0 | 5,470 |
| matlab-platemo | 9,222 | 0 | 5,438 |
| gleam-compiler | 94,816 | 0 | 5,361 |
| zig-tigerbeetle | 54,564 | 0 | 5,139 |
| vue-hoppscotch | 206,584 | 0 | 5,103 |
| kotlin-komga | 63,267 | 0 | 5,097 |
| haskell-pandoc | 96,954 | 0 | 4,951 |
| dart-serverpod | 195,436 | 0 | 4,818 |
| kotlin-detekt | 74,297 | 0 | 4,635 |
| scala-finatra | 61,966 | 0 | 4,499 |
| robot-browser | 12,360 | 0 | 4,293 |
| swift-package-index | 49,089 | 0 | 4,223 |
| python-paperless-ngx | 81,595 | 0 | 4,207 |
| astro-starlight | 20,112 | 0 | 3,956 |
| java-spring-boot-admin | 35,569 | 0 | 3,886 |
| swift-alamofire | 35,633 | 0 | 3,832 |
| java-recaf | 117,103 | 0 | 3,811 |
| perl-perl5 | 125,569 | 0 | 3,766 |
| elixir-changelog | 43,623 | 0 | 3,757 |
| nim-nimbus | 46,166 | 0 | 3,694 |
| ts-trpc | 77,503 | 0 | 3,654 |
| scala-gatling | 99,957 | 0 | 3,621 |
| prolog-scryer | 42,952 | 0 | 3,571 |
| react-tanstack-query | 92,980 | 0 | 3,354 |
