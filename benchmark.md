# BearWisdom Quality Baseline

**Version:** 0.2.0-dev
**Captured:** 2026-03-27
**Projects:** 32

**Totals:** 76,127 files | 608,473 symbols | 1,238,117 edges | 18,380 routes | 7,801 flow edges

## Dedicated Extractors (19 languages)

C#, TypeScript/TSX, Rust, Python, Go, Java, JavaScript/JSX, PHP, Ruby,
Kotlin, Swift, Scala, Dart, Elixir, C, C++, Bash.

## Connectors (21)

**Route detection:** ASP.NET minimal API (with MapGroup), ASP.NET MVC, Spring MVC,
Django URLs, FastAPI, NestJS, Rails, Laravel, Go (stdlib/Gin/Echo/Chi/Mux).

**Flow edges:** .NET DI, .NET Events, Spring DI, Angular DI, Django models/views,
EF Core, gRPC, GraphQL, HTTP client calls (frontend + .NET), React patterns,
Tauri IPC, Electron IPC, message queues.

## Index Statistics

| Project | Files | Symbols | Edges | Routes | Flow Edges | Unresolved |
|---------|------:|--------:|------:|-------:|-----------:|-----------:|
| python-posthog | 15,797 | 192,564 | 442,409 | 1,025 | 3,963 | 168,221 |
| go-mattermost | 7,830 | 75,054 | 238,430 | 68 | 6 | 122,579 |
| grandnode | 3,336 | 61,929 | 68,658 | 986 | 1,338 | 49,019 |
| ruby-discourse | 13,374 | 59,925 | 73,712 | 6,096 | 0 | 127,841 |
| Smartstore | 3,958 | 54,597 | 92,247 | 1,138 | 150 | 45,630 |
| go-gitea | 3,311 | 26,722 | 99,059 | 3,505 | 0 | 59,078 |
| react-calcom | 3,540 | 23,597 | 12,496 | 756 | 878 | 29,909 |
| go-photoprism | 3,468 | 18,996 | 99,133 | 825 | 0 | 27,818 |
| SimplCommerce | 1,477 | 16,773 | 11,828 | 344 | 330 | 35,254 |
| ts-immich | 1,467 | 15,296 | 9,777 | 36 | 198 | 19,304 |
| go-pocketbase | 577 | 14,179 | 17,891 | 582 | 0 | 17,134 |
| python-paperless-ngx | 961 | 11,773 | 14,728 | 929 | 360 | 23,047 |
| react-refine | 7,979 | 8,635 | 7,900 | 0 | 0 | 12,980 |
| php-monica | 1,749 | 8,356 | 20,894 | 1,530 | 0 | 13,173 |
| rust-lemmy | 1,406 | 4,720 | 18,170 | 0 | 0 | 13,997 |
| eShop | 675 | 3,551 | 2,903 | 44 | 360 | 6,177 |
| dotnet-practical-aspnetcore | 1,551 | 2,805 | 1,181 | 125 | 24 | 4,275 |
| angular-ngx-admin | 445 | 1,233 | 372 | 0 | 24 | 2,086 |
| java-petclinic-reactjs | 162 | 983 | 1,304 | 5 | 2 | 2,851 |
| vue-vben-admin | 515 | 868 | 758 | 0 | 0 | 2,216 |
| java-petclinic-rest | 124 | 848 | 1,155 | 5 | 2 | 2,808 |
| full-stack-fastapi-template | 180 | 827 | 677 | 90 | 0 | 1,227 |
| dotnet-CleanArchitecture | 196 | 763 | 615 | 6 | 17 | 1,464 |
| react-tanstack-query | 1,137 | 736 | 270 | 0 | 0 | 1,301 |
| dotnet-EquinoxProject | 123 | 680 | 391 | 20 | 30 | 1,964 |
| angular-coreui | 213 | 568 | 270 | 0 | 0 | 1,286 |
| python-cookiecutter-django | 138 | 486 | 218 | 92 | 84 | 883 |
| java-spring-petclinic | 81 | 337 | 407 | 80 | 0 | 1,268 |
| vue-element-admin | 151 | 250 | 69 | 0 | 0 | 303 |
| ts-nestjs-realworld | 43 | 157 | 76 | 84 | 5 | 297 |
| Test.React | 64 | 143 | 60 | 9 | 30 | 171 |
| vue-vuestic-admin | 99 | 122 | 59 | 0 | 0 | 95 |

## Flow Edge Breakdown

| Project | di_binding | django_model | django_view | event_handler | http_call |
|---------|----------:|-------------:|------------:|--------------:|----------:|
| python-posthog | 0 | 415 | 2,675 | 0 | 873 |
| grandnode | 1,254 | 0 | 0 | 0 | 84 |
| react-calcom | 662 | 0 | 0 | 0 | 216 |
| python-paperless-ngx | 0 | 66 | 294 | 0 | 0 |
| eShop | 144 | 0 | 0 | 108 | 108 |
| SimplCommerce | 330 | 0 | 0 | 0 | 0 |
| ts-immich | 194 | 0 | 4 | 0 | 0 |
| Smartstore | 150 | 0 | 0 | 0 | 0 |
| python-cookiecutter-django | 0 | 0 | 84 | 0 | 0 |
| dotnet-EquinoxProject | 30 | 0 | 0 | 0 | 0 |
| Test.React | 12 | 0 | 0 | 0 | 18 |
| angular-ngx-admin | 24 | 0 | 0 | 0 | 0 |
| dotnet-practical-aspnetcore | 24 | 0 | 0 | 0 | 0 |
| dotnet-CleanArchitecture | 17 | 0 | 0 | 0 | 0 |
| go-mattermost | 0 | 0 | 0 | 0 | 6 |
| ts-nestjs-realworld | 5 | 0 | 0 | 0 | 0 |
| java-petclinic-reactjs | 2 | 0 | 0 | 0 | 0 |
| java-petclinic-rest | 2 | 0 | 0 | 0 | 0 |
