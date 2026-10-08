# Money Manager Telegram Bot — Diseño (SDD fase 2)

Implementa `requirements.md`. Los IDs `RF-*`, `RNF-*`, `BK-*` y `D*` refieren a ese documento.

## 1. Arquitectura

```
Telegram ──long polling──▶ teloxide Dispatcher
                              │
              ┌───────────────┼──────────────────────┐
              ▼               ▼                      ▼
        guard (chat      dialogue FSM           callback queries
        privado,         (SqliteStorage)        (botones inline)
        allowlist)            │                      │
              └───────────────┴──────────┬───────────┘
                                         ▼
                           handlers/*  ──▶ services (parseo, matching, formato)
                                         │
                           SessionManager ── SQLite: sesiones, JWT, credenciales cifradas (AES-256-GCM)
                                         │   re-login automático con POST /auth/login
                                         ▼
                           ApiClient (reqwest) ──HTTP/JSON + Bearer JWT──▶ money_manager_backend /api/v1
```

- El backend no tiene ningún concepto de Telegram. El bot es un cliente HTTP como cualquier otro.
- `ApiClient` es la única capa que conoce HTTP. Los handlers no arman URLs.
- `services` contiene lógica pura, testeable sin red.
- La identidad Telegram → usuario del backend vive solo en el SQLite del bot.

## 2. Backend

El bot usa solo endpoints genéricos existentes: `POST /auth/login`, `POST /auth/logout`, `GET /users/{uuid}`, CRUD de `accounts`, `categories`, `transactions`, `currencies`, `transaction-types`, y `GET /reports/*`. No hay cambios en el backend específicos de Telegram.

Nota: en el primer login desde el host del bot, el backend envía su email de auditoría "Login from unrecognized IP address" al usuario. Es esperado.

## 3. Frontend

`money_manager_frontend` está desmantenido y no se modifica (D7). Consecuencias de los cambios genéricos del backend sobre él:
- su logout no llama a `/auth/logout`, así que no revoca su token (BK-5);
- si se deja la cuenta vacía en una transacción, recibe `403` (BK-8);
- los JWT que tenga guardados de antes del deploy dejan de valer (no tienen `jti`).

## 4. Configuración del bot

Todas por variables de entorno. Se cargan desde `env/.env` con `dotenvy` (igual que el backend). Se documentan en `env/.env.example` y en el `README.md` del bot.

| Variable | Obligatoria | Default | Descripción |
|----------|-------------|---------|-------------|
| `TELOXIDE_TOKEN` | sí | — | token de @BotFather |
| `BACKEND_API_URL` | no | `http://127.0.0.1:8000/api/v1` | URL base de la API |
| `STORE_CREDENTIALS` | no | `true` | guarda las credenciales cifradas para re-loguear solo (D8). `false`: el bot pide `/login` cada vez que vence el JWT |
| `CREDENTIALS_KEY` | sí, si `STORE_CREDENTIALS=true` | — | clave AES-256 en base64 (32 bytes). Generar con `openssl rand -base64 32`. Si se pierde o cambia, las credenciales guardadas no se pueden descifrar y cada usuario debe hacer `/login` de nuevo |
| `ALLOWED_TELEGRAM_IDS` | no | vacío = todos | IDs numéricos de Telegram separados por coma, p. ej. `123456789,987654321`. Para conocer el propio ID: escribirle a @userinfobot |
| `DATABASE_PATH` | no | `data/bot.sqlite` | archivo SQLite de sesiones y diálogos. Se crea con permisos `0600` si no existe |
| `BOT_TZ` | no | `America/Argentina/Buenos_Aires` | zona horaria IANA para interpretar y mostrar fechas |
| `CACHE_TTL_SECS` | no | `300` | vida de la caché de catálogos |
| `PAGE_SIZE` | no | `10` | filas por página en listados |
| `HTTP_TIMEOUT_SECS` | no | `10` | timeout de cada request al backend |
| `RUST_LOG` | no | `info` | filtro de `tracing` |

`env/.env.example`:

```dotenv
TELOXIDE_TOKEN=123456:ABC-replace-me
BACKEND_API_URL=http://127.0.0.1:8000/api/v1
# Store credentials encrypted so the bot can re-login when the JWT expires.
STORE_CREDENTIALS=true
# Required when STORE_CREDENTIALS=true. Generate with: openssl rand -base64 32
CREDENTIALS_KEY=
# Optional. Comma-separated Telegram user IDs. Empty = everyone.
ALLOWED_TELEGRAM_IDS=
DATABASE_PATH=data/bot.sqlite
BOT_TZ=America/Argentina/Buenos_Aires
CACHE_TTL_SECS=300
PAGE_SIZE=10
HTTP_TIMEOUT_SECS=10
RUST_LOG=info
```

Al arrancar, el bot falla con un mensaje claro si falta una variable obligatoria, si `CREDENTIALS_KEY` no decodifica a 32 bytes o si `BOT_TZ` no es válida.

## 5. Estructura del crate

```
money_manager_telegram/
├── Cargo.toml
├── Dockerfile
├── README.md
├── env/.env.example
├── migrations/                # sqlx migrations del bot (SQLite)
│   └── 0001_init.sql
├── specs/                     # este SDD
├── src/
│   ├── main.rs                # arranque, migraciones, Dispatcher
│   ├── config.rs              # BotConfig::from_env()
│   ├── errors.rs              # BotError + traducción a mensajes (§8)
│   ├── api/
│   │   ├── mod.rs
│   │   ├── client.rs          # ApiClient: auth, users, accounts, categories, transactions, currencies, types, reports
│   │   ├── models.rs          # DTOs espejo de money_manager_backend/src/models
│   │   └── error.rs           # ApiError { status, code, message }
│   ├── session/
│   │   ├── mod.rs             # SessionManager: token_for(user), invalidate, catalog
│   │   ├── store.rs           # SQLite (sqlx): sessions, refs, list_state, login_failures
│   │   ├── crypto.rs          # AES-256-GCM de credenciales
│   │   └── catalog.rs         # caché en memoria de catálogos
│   ├── bot/
│   │   ├── mod.rs             # schema() de dptree
│   │   ├── commands.rs        # enum Command (BotCommands) + comandos por rol
│   │   ├── state.rs           # enum State (Serialize/Deserialize para SqliteStorage)
│   │   ├── callbacks.rs       # enum Callback + encode/decode de callback_data
│   │   ├── keyboards.rs
│   │   ├── format.rs
│   │   └── handlers/
│   │       ├── auth.rs        # /start /help /login /logout /me /cancel
│   │       ├── transactions.rs
│   │       ├── accounts.rs
│   │       ├── categories.rs
│   │       ├── currencies.rs
│   │       ├── types.rs
│   │       └── reports.rs     # /balance /summary
│   └── services/
│       ├── parse.rs
│       └── matching.rs
└── src/live_tests.rs         # test #[ignore] contra un backend real (MM_LIVE_URL)

Los tests unitarios y de integración (wiremock, SQLite en memoria) viven en módulos `#[cfg(test)]`
junto al código: el crate es un binario y `tests/` no ve sus módulos internos.
```

## 6. Dependencias del bot

| Crate | Uso |
|-------|-----|
| `teloxide` (features `macros`, `rustls`, `sqlite-storage-rustls`; sin default features) | Bot API, `BotCommands`, `Dispatcher`, `SqliteStorage` |
| `sqlx` (`sqlite`, `runtime-tokio`, `migrate`) | tablas propias del bot en el mismo archivo SQLite |
| `tokio` (`rt-multi-thread`, `macros`) | runtime |
| `reqwest` (`json`, `rustls-tls`; sin default features) | cliente HTTP |
| `serde`, `serde_json` | DTOs y estado de diálogos |
| `rust_decimal` | montos (nunca `f64`) |
| `chrono`, `chrono-tz` | fechas y `BOT_TZ` |
| `uuid` (`serde`) | IDs |
| `base64` | leer `exp` del payload del JWT; decodificar `CREDENTIALS_KEY` |
| `aes-gcm`, `zeroize` | cifrado de credenciales en reposo y borrado de secretos en memoria |
| `thiserror` | errores |
| `tracing`, `tracing-subscriber` | logs |
| `dotenvy` | `env/.env` |
| `unicode-normalization` | matching sin acentos |
| dev: `wiremock` | tests |

Versiones exactas: última estable de cada una al crear `Cargo.toml`. `teloxide` y `sqlx` deben compartir la misma versión mayor de `sqlx` (la que usa `teloxide`).

## 7. Modelo de datos del bot

### 7.1 SQLite (`migrations/0001_init.sql`)

`SqliteStorage` de teloxide crea su propia tabla `teloxide_dialogues`. Las tablas del bot:

```sql
CREATE TABLE sessions (
    telegram_user_id   INTEGER PRIMARY KEY,
    chat_id            INTEGER NOT NULL,
    user_uuid          TEXT NOT NULL,
    username           TEXT NOT NULL,
    role               TEXT NOT NULL,
    jwt                TEXT,                  -- NULL cuando venció y no hay credenciales
    jwt_expires_at     INTEGER,               -- unix seconds
    credentials        BLOB,                  -- nonce (12 bytes) || AES-256-GCM(identifier \0 password); NULL si STORE_CREDENTIALS=false
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE TABLE short_refs (
    telegram_user_id   INTEGER NOT NULL,
    kind               TEXT NOT NULL,         -- transaction | account | category | currency | type
    position           INTEGER NOT NULL,      -- 1..n
    target             TEXT NOT NULL,         -- UUID o código de moneda
    PRIMARY KEY (telegram_user_id, kind, position)
);

CREATE TABLE list_state (
    telegram_user_id   INTEGER NOT NULL,
    kind               TEXT NOT NULL,
    filters_json       TEXT NOT NULL,         -- filtros del último listado, para la paginación por botones
    PRIMARY KEY (telegram_user_id, kind)
);

CREATE TABLE login_failures (
    telegram_user_id   INTEGER NOT NULL,
    failed_at          INTEGER NOT NULL
);
```

Cifrado de credenciales (`session/crypto.rs`):
- AES-256-GCM (`aes-gcm`), nonce aleatorio de 96 bits por escritura (`OsRng`).
- AAD = `telegram_user_id` en big-endian: una fila copiada a otro usuario no descifra.
- Texto plano = `identifier` + `\0` + `password`, envuelto en `Zeroizing<String>` al descifrar.

El JWT se guarda en claro: vence en 24 h, el archivo es `0600`, y quien lea el archivo y tenga `CREDENTIALS_KEY` (variable de entorno del mismo proceso) obtiene igual las credenciales.

### 7.2 Obtención del JWT (`SessionManager::token_for`)

```
token_for(telegram_user_id):
  s = sessions[telegram_user_id]                       (si no hay → BotError::NotLoggedIn)
  si s.jwt y s.jwt_expires_at > now + 60s → s.jwt
  si s.credentials:
    (identifier, password) = decrypt(s.credentials)
    r = POST /auth/login
    200 → update sessions(jwt, exp, role) → r.token
    401/400 → borrar jwt y credentials → BotError::SessionExpired
  si no → borrar jwt → BotError::SessionExpired
```

- `ApiClient` ante `401` con JWT: invalida `jwt_expires_at`, llama `token_for` y reintenta una vez. Un segundo `401` → `BotError::SessionExpired`.
- `exp` se lee del payload del JWT (base64, sin verificar firma: el bot no tiene la clave y solo lo usa para saber cuándo renovar).
- Un `Mutex` por `telegram_user_id` evita varios re-login en paralelo para el mismo usuario.
- El menú por rol (`setMyCommands`) se actualiza en `/login`, `/start`, `/logout` y al vencer la sesión (RF-01.12).

### 7.3 DTOs (`api/models.rs`)

Espejo 1:1 de `money_manager_backend/src/models/*.rs`:

- `LoginResponse { token, user: User }`
- `User`, `Account`, `Category`, `Currency`, `TransactionType`
- `Transaction { amount: Decimal (desde string), transaction_date: DateTime<Utc>, category_uuid: Option<Uuid>, account_uuid: Option<Uuid>, ... }`
- `Paginated<T> { data, meta: Meta { total_records, current_page, total_pages, per_page } }`
- Requests de alta y edición con `skip_serializing_if = "Option::is_none"`. En `UpdateTransaction`, `category_uuid` y `account_uuid` son `Option<Option<Uuid>>`: `None` = no se envía, `Some(None)` = `null` (BK-2).
- `amount` se serializa como string con punto decimal (`Decimal::normalize()`).

### 7.4 Caché de catálogos (memoria)

```rust
pub struct Catalog {
    pub accounts: Vec<Account>,
    pub categories: Vec<Category>,
    pub currencies: Vec<Currency>,
    pub types: Vec<TransactionType>,
    pub type_in: Option<Uuid>,    // code == "IN"
    pub type_out: Option<Uuid>,   // code == "OUT"
    pub loaded_at: Instant,
}
```

- Se carga con `tokio::try_join!` de los 4 listados (`per_page=100`, paginando).
- Clave: `telegram_user_id`. TTL `CACHE_TTL_SECS`. Se invalida en todo alta, edición o baja de cuentas, categorías, monedas o tipos.
- Sin `type_out`/`type_in`, `/expense` e `/income` responden con error claro y sugieren `/new`.

## 8. Errores (`errors.rs`)

`ApiError` se arma desde el body `{ "error": CODE, "message": "..." }` del backend.

| Origen | Mensaje al usuario | Efecto |
|--------|--------------------|--------|
| `BotError::NotLoggedIn` | "Iniciá sesión con /login." | — |
| `BotError::SessionExpired` | "Tu sesión venció. Enviá /login para volver a entrar." | borra JWT y credenciales |
| `/login` → `401` o `400` | "Usuario o contraseña incorrectos." | cuenta fallo (RF-01.11) |
| `/login` sobre el límite | "Demasiados intentos, esperá N minutos." | — |
| `403 FORBIDDEN` | "No tenés permisos para esa operación." | — |
| `404 NOT_FOUND` | "No encontré ese registro (¿fue borrado?)." | invalida caché |
| `400 BAD_REQUEST` | "Datos inválidos: {message}" | — |
| `409 CONFLICT` | "{message}" | — |
| `5xx`, timeout o conexión | "El servidor no responde, probá en un rato." | log `error` |
| Error de parseo local | mensaje específico + ejemplo de uso | — |

Los handlers devuelven `Result<(), BotError>`. Un único `error_handler` envía el mensaje y loguea. Nunca se propaga texto crudo de reqwest.

## 9. Interacción

### 9.1 Comandos (`bot/commands.rs`)

`#[derive(BotCommands)] #[command(rename_rule = "snake_case")]`. Telegram solo admite `[a-z0-9_]`.

| Comando | Args | Rol mínimo | Diálogo si faltan args |
|---------|------|------------|------------------------|
| `start`, `help`, `cancel` | — | ninguno | — |
| `login` | `[username\|email]` | ninguno | pide username o email y luego la contraseña |
| `logout`, `me` | — | con sesión | — |
| `expense`, `income` | `<amount> [date] [#cat] [@account] [desc]` | `user` | sin monto → asistente `/new` con tipo preseleccionado |
| `new` | — | `user` | asistente completo |
| `transactions` | filtros (RF-02.7) | `auditor` | — |
| `show` | `<ref\|uuid>` | `auditor` | — |
| `edit`, `delete` | `<ref\|uuid>` | `user` | — |
| `accounts`, `categories`, `currencies`, `types`, `balance`, `summary` | ver RF | `auditor` | — |
| `account_new` | `<name> <CURRENCY>` | `user` | pide nombre y moneda (teclado) |
| `account_edit`, `account_delete`, `account_default` | `<ref>` | `user` | teclado de cuentas |
| `category_new` | `<name>` | `user` | pide nombre |
| `category_edit`, `category_delete` | `<ref>` | `user` | teclado de categorías |
| `currency_new` | `<CODE> <symbol> <name…>` | `manager` | muestra el uso |
| `currency_edit`, `currency_delete` | `<CODE>` | `manager` | teclado de monedas |
| `type_new` | `<code> <name…>` | `manager` | muestra el uso |
| `type_edit`, `type_delete` | `<ref>` | `manager` | teclado de tipos |

Lectura de la columna "Rol mínimo":
- `auditor`: lo ven `auditor`, `user`, `manager` y `admin`;
- `user`: lo ven `user`, `manager` y `admin` (no `auditor`, que es solo lectura);
- `manager`: lo ven `manager` y `admin`.

`commands_for(role)` arma la lista explícitamente por rol, espejo de la política Casbin de `money_manager_backend/src/services/authorization.rs`. El backend sigue siendo la autoridad: el filtro del bot es solo UX.

### 9.2 Gramática de alta rápida (`services/parse.rs`)

```
expense  := amount [date] token*
amount   := dígitos con separadores (RF-02.4)
date     := "today" | "hoy" | "yesterday" | "ayer" | dd/mm | dd/mm/yyyy
token    := "#" palabra        → categoría
          | "@" palabra        → cuenta
          | palabra            → descripción (se concatenan en orden)
```

Reglas de monto:
- Si hay `.` y `,`, el último que aparece es el separador decimal y el otro es de miles.
- Si hay un solo tipo de separador y aparece una vez seguido de 1 o 2 dígitos, es decimal. Seguido de exactamente 3 dígitos, es de miles (`1.500` = 1500).
- Más de 2 decimales, cero, negativo o no numérico → "Monto inválido".

Casos de test obligatorios:

| Entrada | Resultado |
|---------|-----------|
| `1500` | 1500 |
| `1500,5` | 1500.5 |
| `1.500` | 1500 |
| `1.500,50` | 1500.50 |
| `1,500.50` | 1500.50 |
| `12.5` | 12.5 |
| `0` / `-3` / `1,234,5` / `abc` | error |

Fechas en `BOT_TZ`. `today` = ahora. `yesterday` o fecha explícita = 12:00 local de ese día, para evitar corrimientos de día al pasar a UTC. Fecha futura → se acepta con aviso.

Filtros de `/transactions`: `@account`, `#category`, `expenses`, `income`, `month:10/2026`, `from:01/10`, `to:15/10/2026`. El resto de las palabras se une como `search`.

### 9.3 Diálogos (`bot/state.rs`)

`State` deriva `Serialize`/`Deserialize` para `SqliteStorage` con `serializer::Json`. Ningún estado guarda secretos: la contraseña se lee en el handler de `LoginAskPassword`, se usa y se descarta sin pasar por el estado.

```rust
#[derive(Clone, Default, Serialize, Deserialize)]
pub enum State {
    #[default]
    Idle,
    LoginAskIdentifier,
    LoginAskPassword { identifier: String },
    TxWizard { draft: TxDraft, step: TxStep },
    TxEditValue { uuid: Uuid, field: TxField },
    AccountCreate { name: Option<String> },
    AccountEditValue { uuid: Uuid, field: AccountField },
    CategoryCreate,
    CategoryEditValue { uuid: Uuid, field: CategoryField },
    CurrencyCreate { draft: CurrencyDraft },
    CurrencyEditValue { code: String, field: CurrencyField },
    TypeCreate { draft: TypeDraft },
    TypeEditValue { uuid: Uuid, field: TypeField },
}
```

- `TxStep`: `Type → Amount → Category → Account → Date → Description → Confirm`. Los pasos con opciones cerradas se responden con botones; el resto con texto. Botón "Omitir" en descripción.
- `/cancel` y cualquier comando nuevo hacen `dialogue.exit()`.

### 9.4 Listados y orden

`/transactions` pide al backend una página por vez (`page`, `per_page=PAGE_SIZE`, `sort=-transaction_date`) con los filtros traducidos a query params. Guarda los filtros en `list_state` para que ◀ ▶ funcionen tras un reinicio. Total de páginas desde `meta.total_pages`.

### 9.5 Callbacks (`bot/callbacks.rs`)

`callback_data` ≤ 64 bytes, formato `<acción>:<arg>`. Un UUID ocupa 36 bytes.

| Callback | Acción |
|----------|--------|
| `te:<uuid>` / `tf:<field>:<uuid>` | editar transacción / elegir campo |
| `td:<uuid>` / `tdy:<uuid>` | borrar transacción / confirmar |
| `wt:in` `wt:out` `wt:<uuid>` | tipo en el asistente |
| `wc:<uuid>` `wc:none` `wc:more:<page>` | categoría en el asistente |
| `wa:<uuid>` | cuenta en el asistente |
| `wd:today` `wd:yday` | fecha en el asistente |
| `ok` / `no` | confirmar / cancelar |
| `pg:<kind>:<page>` | paginación |
| `ae:` `ad:` `adf:` + `<uuid>` | cuenta: editar / borrar / por defecto |
| `ce:` `cd:` + `<uuid>` | categoría: editar / borrar |
| `ye:` `yd:` + `<CODE>` | moneda: editar / borrar |
| `ke:` `kd:` + `<uuid>` | tipo: editar / borrar |
| `nc:<nombre ≤ 40 bytes>` | crear categoría inexistente (RF-02.3) |
| `lo:y` | confirmar `/logout` |

Todo callback llama `answer_callback_query` y edita el mensaje original cuando corresponde.

### 9.6 Formato de salida (`bot/format.rs`)

- `ParseMode::Html`, con escape de todo texto del usuario (`teloxide::utils::html::escape`).
- Monto: `− $ 1.500,50` / `+ US$ 200,00`, con el símbolo de `Currency.symbol`. Si dos monedas en pantalla comparten símbolo, se agrega el código (`$ 1.500,50 ARS`).
- Fecha: `08/10 14:32` en el año actual, `08/10/2025` en otros años.
- Fila: `3. 08/10 − $ 1.500,50 · Coffee · cortado con medi…`

## 10. Lógica de servicios

### 10.1 Saldo (RF-06.1)

`GET /reports/balance[?account_uuid=…]` devuelve por cuenta `income`, `expense`, `other`, `balance` y `transaction_count`, más `totals` por moneda. `other` (tipos distintos de `IN`/`OUT`) se muestra aparte y no entra en el saldo. El bot solo formatea.

### 10.2 Resumen mensual (RF-06.2)

1. Calcula el inicio del mes y el inicio del mes siguiente en `BOT_TZ` y los convierte a UTC (RFC 3339 con `Z`).
2. `GET /reports/by-category?from=…&to=…&account_uuid=…` (por defecto `type_code=OUT`): filas por categoría y moneda, ya ordenadas por total desc, con `percentage`. `category_uuid = null` → "Sin categoría".
3. `GET /reports/by-category?type_code=IN&…` para el total de ingresos del mes.
4. Muestra hasta 10 categorías y agrupa el resto en "Otras". Neto = ingresos − gastos, por moneda.

## 11. Seguridad

- **Contraseña en el chat**: pasa por Telegram una vez, en el `/login`. El bot borra el mensaje apenas lo lee (RF-01.4). Telegram la ve en tránsito; es el costo de no tener frontend.
- **Credenciales en reposo**: cifradas con AES-256-GCM. `CREDENTIALS_KEY` vive solo en el entorno del proceso, nunca en el SQLite, en logs ni en la imagen Docker. Quien tenga el archivo SQLite **y** la clave puede recuperar contraseñas: proteger el host del bot como se protege el backend. Con `STORE_CREDENTIALS=false` no se guardan contraseñas.
- **Rotar `CREDENTIALS_KEY`**: invalida todas las credenciales guardadas (no se pueden descifrar); los usuarios hacen `/login` de nuevo cuando venza su JWT.
- **Riesgo aceptado**: quien controle la cuenta de Telegram del usuario controla sus finanzas en el bot. El README recomienda activar la verificación en dos pasos de Telegram.
- `/logout` revoca el JWT en el backend (BK-5) y borra JWT y credenciales del SQLite.
- Solo `ChatKind::Private`; en grupos el bot no responde.
- Allowlist evaluada antes de cualquier handler.
- `tracing` nunca registra bodies, JWT, contraseñas ni claves. `Debug` de `Session` y `BotConfig` los oculta.
- Fuera de la red local, `BACKEND_API_URL` debe ser `https://`. Si no lo es y el host no es `localhost`, `127.0.0.1` o un nombre de servicio Docker, el bot loguea `warn` al arrancar.

## 12. Testing

| Nivel | Qué | Herramienta |
|-------|-----|-------------|
| Unit (bot) | `parse`, `matching`, `format`, callbacks ida y vuelta, `commands_for(role)` | `cargo test` |
| Unit (bot) | cifrado: ida y vuelta, AAD de otro usuario falla, clave incorrecta falla | `cargo test` |
| Integración (bot) | `ApiClient` con fixtures: login, re-login ante `401` y por `exp`, paginación, errores, PATCH parcial | `wiremock` |
| Integración (bot) | `session::store` contra SQLite en memoria | `sqlx` |
| Backend | `UpdateTransactionDto` ausente vs `null` (hecho); BK-1 a BK-7 verificados contra la API en vivo (hecho) | `cargo test` |
| E2E manual | checklist con bot de prueba, backend en Docker y usuario seed `default` | guía en `README.md` |

## 13. Decisiones (ADR resumidas)

| # | Decisión | Alternativa descartada | Motivo |
|---|----------|------------------------|--------|
| 1 | `teloxide` + `dptree` | `frankenstein` | más maduro; diálogos, `SqliteStorage` y `BotCommands` integrados |
| 2 | Long polling | webhooks | sin dominio ni TLS propio; migrable luego |
| 3 | Login con `POST /auth/login` desde el chat y credenciales cifradas en el bot | endpoints de vinculación Telegram en el backend | el backend no debe saber de Telegram; todo lo de Telegram vive en el bot |
| 4 | Re-login automático con credenciales cifradas (configurable) | pedir la contraseña cada 24 h | UX de "loguearse una vez" sin cambiar el backend (D8) |
| 5 | SQLite para sesiones y diálogos | memoria / Redis | sobrevive reinicios sin infraestructura extra (D3) |
| 6 | Agregaciones en el backend (`/reports/*`) | calcular en el bot | SQL agrega sin traer todas las filas; endpoint genérico, sirve a cualquier cliente (BK-4) |
| 7 | Comandos en inglés, textos en español | todo en un idioma | D6 |
| 8 | `rust_decimal` | `f64` | exactitud; el backend usa `Decimal` |
| 9 | Crate independiente | workspace con el backend | el backend no es workspace; el bot es cliente HTTP y duplica DTOs a propósito |
