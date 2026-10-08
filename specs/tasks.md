# Money Manager Telegram Bot — Tareas (SDD fase 3)

Estado al 2026-10-08: fases 0 a 9 implementadas; 47 tests (unitarios + wiremock) y un test en vivo contra el backend pasan; falta T9.4.

Cada tarea referencia requisitos (`RF-*`, `RNF-*`) y secciones de `design.md`. Una tarea está terminada
cuando compila sin warnings (`cargo clippy -- -D warnings`), tiene tests y pasa `cargo test`.

## Fase 0 — Backend: correcciones

- [x] T0.1 BK-1: autorización por dueño en `transactions` `list`, `get_one`, `create`, `update` y `delete`; validación de cuenta destino y categoría.
- [x] T0.2 BK-2: `UpdateTransactionDto` con `Option<Option<Uuid>>` para `category_uuid` y `account_uuid` + test unitario.
- [x] T0.3 BK-3: `sort` en `GET /transactions`.
- [x] T0.4 BK-4: `GET /reports/balance` y `GET /reports/by-category`.
- [x] T0.5 BK-5: `jti`, tabla `revoked_tokens`, logout que revoca, extractor que rechaza revocados.
- [x] T0.6 BK-6: login sin validar política de contraseña.
- [x] T0.7 BK-7: `409` al borrar cuenta con transacciones; categoría se desvincula al borrarse.
- [x] T0.8 Verificación en vivo de BK-1 a BK-7 (29 checks contra Postgres del `docker-compose.yml`).

El backend no se modifica para Telegram (D1): no hay fase de backend para el bot.

## Fase 1 — Bot: esqueleto

- [x] T1.1 `cargo new`, dependencias (§6), cabecera de copyright. (RNF-01, RNF-09)
- [x] T1.2 `config.rs` + `env/.env.example` con todas las variables de §4; validación al arrancar.
- [x] T1.3 SQLite: apertura con `0600`, `sqlx::migrate!`, `SqliteStorage` para diálogos. (RF-01.10)
- [x] T1.4 `main.rs`: tracing, `Dispatcher`, guard de chat privado y allowlist. (RF-01.1, RF-01.2)
- [x] T1.5 `/start`, `/help`, `/cancel`. (RF-07.1, RF-07.2)

## Fase 2 — Bot: cliente API y sesión

- [x] T2.1 `api/models.rs` (§7.3) y `api/error.rs`.
- [x] T2.2 `ApiClient` base: timeout, bearer, parseo de errores, `fetch_all` paginado.
- [x] T2.3 Endpoints de auth, users, CRUD de los 5 recursos y reportes.
- [x] T2.4 `session/crypto.rs` (AES-256-GCM con AAD) + tests. (RF-01.5)
- [x] T2.5 `SessionManager::token_for` con re-login por `exp` y por `401`, mutex por usuario. (§7.2, RF-01.6, RF-01.7)
- [x] T2.6 Caché de catálogos. (§7.4, RNF-03)
- [x] T2.7 Tests `wiremock` y de `session::store`.

## Fase 3 — Bot: login

- [x] T3.1 `/login` en dos pasos (identificador → contraseña), borrado del mensaje, rate limit. (RF-01.3, RF-01.4, RF-01.11)
- [x] T3.2 `/logout` con confirmación, `/me`. (RF-01.8, RF-01.9)
- [x] T3.3 `setMyCommands` por chat según rol (`commands_for(role)`). (RF-01.12)

## Fase 4 — Bot: servicios puros

- [x] T4.1 `parse.rs`: montos, fechas, tokens, filtros. Cobertura ≥ 90 %. (§9.2)
- [x] T4.2 `matching.rs`.
- [x] T4.3 `format.rs`. (RNF-05)
- [x] T4.4 `callbacks.rs` con tests de ida y vuelta. (§9.5)

## Fase 5 — Bot: transacciones

- [x] T5.1 `/expense`, `/income`. (RF-02.1, RF-02.4, RF-02.6)
- [x] T5.2 Teclado de categorías y creación inline. (RF-02.2, RF-02.3)
- [x] T5.3 Asistente `/new`. (RF-02.5)
- [x] T5.4 `/transactions` con filtros, orden local y paginación persistida. (RF-02.7, RF-02.8, §9.4)
- [x] T5.5 `/show`, `/edit`, `/delete`. (RF-02.9–RF-02.11)

## Fase 6 — Bot: cuentas y categorías

- [x] T6.1 `/accounts` con saldo. (RF-03.1)
- [x] T6.2 `/account_new`, `/account_edit`, `/account_default`, `/account_delete`. (RF-03.2–RF-03.6)
- [x] T6.3 `/categories`, `/category_new`, `/category_edit`, `/category_delete`. (RF-04)

## Fase 7 — Bot: monedas y tipos

- [x] T7.1 `/currencies`, `/types`. (RF-05.1, RF-05.2)
- [x] T7.2 ABM de monedas para `manager`/`admin`. (RF-05.3, RF-05.6)
- [x] T7.3 ABM de tipos con protección de `IN`/`OUT`. (RF-05.4–RF-05.6)

## Fase 8 — Bot: reportes

- [x] T8.1 `/balance` sobre `GET /reports/balance`. (RF-06.1, §10.1)
- [x] T8.2 `/summary` sobre `GET /reports/by-category`. (RF-06.2, §10.2)

## Fase 9 — Entrega

- [x] T9.1 `Dockerfile` multi-stage y `docker-compose.yml` propio del bot con volumen para `DATABASE_PATH`. (RNF-08)
- [x] T9.2 `README.md` del bot: @BotFather, generación de `CREDENTIALS_KEY`, tabla de variables, cómo obtener el Telegram ID, ejemplos de comandos, recomendación de 2FA en Telegram, checklist E2E.
- [ ] T9.4 Ejecutar el checklist E2E del README con un bot real de Telegram (requiere `TELOXIDE_TOKEN`).
- [x] T9.3 Revisión de seguridad: logs sin secretos, permisos del SQLite, allowlist, rate limits. (RNF-02)

## Post-MVP

- [ ] P1 Recordatorios programados (resumen semanal automático).
