# Money Manager Telegram Bot — Requisitos (SDD fase 1)

## 1. Objetivo

Bot de Telegram escrito en Rust que permite hacer el ABM (alta, baja, modificación) y la consulta
de las finanzas personales del usuario contra la API de `money_manager_backend` (`/api/v1`).
El bot es un **cliente** de la API, igual que lo era `money_manager_frontend`, optimizado para carga
rápida desde el celular.

El usuario inicia sesión **una sola vez** desde el chat. Después, el bot renueva el JWT solo y el
usuario no vuelve a escribir credenciales (salvo que cambie su contraseña o desactive esa opción).

**Separación de responsabilidades**: el backend es solo backend y no sabe nada de Telegram.
Todo lo relacionado con Telegram (identidad, sesión, credenciales, diálogos) vive en
`money_manager_telegram`, que usa únicamente endpoints genéricos de la API (`POST /auth/login`,
`POST /auth/logout`, CRUD y reportes). `money_manager_frontend` está desmantenido y no se modifica.

## 2. Decisiones tomadas

| # | Decisión |
|---|----------|
| D1 | Multiusuario. Cada Telegram ID inicia sesión con su usuario del backend vía `POST /auth/login`. El backend no se modifica para Telegram. |
| D2 | Los hallazgos del backend (BK-1 a BK-8) se corrigieron en el backend porque son genéricos (seguridad, datos, reportes), no de Telegram (ver §8). |
| D3 | Sesiones, credenciales cifradas y diálogos persistidos en SQLite, dentro del bot. |
| D4 | Allowlist de usuarios de Telegram opcional, configurable por variable de entorno. |
| D5 | El MVP incluye el ABM de monedas y tipos de transacción para roles `admin` y `manager`. |
| D6 | Comandos en inglés. Los textos de respuesta quedan en español (es-AR). |
| D7 | `money_manager_frontend` está desmantenido: no se modifica. |
| D8 | Para no pedir la contraseña cada 24 h (vida del JWT), el bot guarda las credenciales cifradas (AES-256-GCM) y re-loguea solo. Se puede desactivar con `STORE_CREDENTIALS=false`: en ese caso el bot pide `/login` cuando vence el JWT. |
| D9 | `AUTH_METHOD` elige el método de login: `password` (default, D1/D8) o `telegram`. Con `telegram` no hay contraseña: la identidad es el Telegram ID, mapeado a un usuario del backend en `TELEGRAM_USERS`, y el bot firma los JWT con el `JWT_SECRET` del backend. El backend no se modifica. |

## 3. Alcance

### Dentro del alcance (MVP)

| Recurso backend          | Operaciones en el bot                          | Roles |
|--------------------------|------------------------------------------------|-------|
| `auth`                   | login (una vez), logout, perfil                | todos |
| `transactions`           | alta (rápida y guiada), listado, detalle, edición, baja | `user`, `manager`, `admin` |
| `accounts`               | alta, listado, edición, baja, marcar por defecto | `user`, `manager`, `admin` |
| `categories`             | alta, listado, edición, baja                   | `user`, `manager`, `admin` |
| `currencies`             | listado (todos); alta, edición, baja (`admin`, `manager`) | ver §5 RF-05 |
| `transaction-types`      | listado (todos); alta, edición, baja (`admin`, `manager`) | ver §5 RF-05 |
| `reports`                | saldo por cuenta, resumen mensual por categoría | todos |

El rol `auditor` solo puede usar los comandos de lectura (lo impone la política Casbin del backend).

### Fuera del alcance (MVP)

- Cualquier cambio en `money_manager_backend` específico de Telegram.
- Cualquier cambio en `money_manager_frontend` (desmantenido).
- `users` (ABM de usuarios) y `audit-logs`: administración.
- Registro, verificación de email, forgot/reset password y cambio de contraseña.
- Grupos de Telegram, modo inline y webhooks (el MVP usa long polling).

## 4. Actores y glosario

- **Usuario final**: persona con cuenta en Money Manager que habla con el bot por chat privado.
- **Operador**: quien despliega el bot (token de Telegram, clave de cifrado, allowlist).
- **Telegram ID**: `from.id` del usuario de Telegram (entero de 64 bits). Telegram lo fija del lado del servidor y no se puede falsificar a través de la Bot API.
- **Sesión**: fila en el SQLite del bot que asocia un Telegram ID con un usuario del backend, su JWT vigente y, opcionalmente, sus credenciales cifradas.
- **Clave de credenciales**: `CREDENTIALS_KEY`, clave AES-256 del bot para cifrar credenciales en reposo.
- **Gasto / Ingreso**: transacción cuyo tipo tiene `code = "OUT"` / `"IN"` (seed: `expenses` / `income`).
- **Cuenta por defecto**: cuenta del usuario con `is_default = true` (seed: `default`, moneda `ARS`).
- **Referencia corta**: número de fila (`1`, `2`, ...) del último listado mostrado en ese chat, alias de un UUID.

## 5. Requisitos funcionales

Formato EARS. Criterios de aceptación en Given/When/Then.

### RF-01 Acceso y sesión

- **RF-01.1** El bot DEBE responder solo en chats privados.
- **RF-01.2** Cuando `ALLOWED_TELEGRAM_IDS` tiene valores, el bot DEBE ignorar a usuarios fuera de la lista (responde "No autorizado" una sola vez por usuario). Vacío o ausente = todos permitidos.
- **RF-01.3** `/login` DEBE pedir usuario o email y luego contraseña, y llamar `POST /auth/login` (`username` o `email` según si el identificador contiene `@`).
- **RF-01.4** El bot DEBE borrar el mensaje con la contraseña apenas lo lee, antes de llamar a la API. Si no puede borrarlo, DEBE pedirle al usuario que lo borre.
- **RF-01.5** Con `STORE_CREDENTIALS=true` (default), el bot DEBE guardar identificador y contraseña cifrados con AES-256-GCM (`CREDENTIALS_KEY`), usando el Telegram ID como dato asociado (AAD) para que una fila no se pueda descifrar como si fuera de otro usuario.
- **RF-01.6** El bot DEBE obtener el JWT sin intervención del usuario cuando falta, vence (`exp` − 60 s) o la API responde `401`: re-loguea con las credenciales guardadas y reintenta la operación una sola vez.
- **RF-01.7** Si el re-login falla con `401` (contraseña cambiada, usuario desactivado), o no hay credenciales guardadas, el bot DEBE borrar el JWT y las credenciales y responder "Tu sesión venció. Enviá /login para volver a entrar."
- **RF-01.8** `/logout` DEBE llamar `POST /auth/logout` (revoca el JWT, BK-5) y borrar la sesión local, incluidas las credenciales.
- **RF-01.9** `/me` DEBE mostrar username, email, rol, email verificado, último login y si hay credenciales guardadas (`GET /users/{uuid}`).
- **RF-01.10** El bot DEBE persistir en SQLite la sesión y el estado de los diálogos, para sobrevivir reinicios sin volver a pedir login.
- **RF-01.11** El bot DEBE limitar los intentos de login fallidos: 5 por Telegram ID cada 15 minutos.
- **RF-01.12** El bot DEBE mostrar en el menú de Telegram (`setMyCommands` con scope por chat) solo los comandos que el rol del usuario puede usar. El menú se actualiza en `/login`, `/start`, `/logout` y cuando la sesión vence.
- **RF-01.13** Todo comando de finanzas sin sesión DEBE responder "Iniciá sesión con /login".
- **RF-01.14** `AUTH_METHOD` DEBE aceptar `password` (default) o `telegram`. Cualquier otro valor DEBE impedir el arranque con un mensaje claro. RF-01.3 a RF-01.7 y RF-01.11 aplican solo a `password`.

#### Método `telegram` (sin contraseña)

- **RF-01.15** Con `AUTH_METHOD=telegram`, `JWT_SECRET` y `TELEGRAM_USERS` (`telegram_id:user_uuid,…`) son obligatorios. `CREDENTIALS_KEY` no se usa.
- **RF-01.16** `/start` y `/login` DEBEN iniciar sesión sin pedir datos si el Telegram ID (`from.id`) está en `TELEGRAM_USERS`. Si no está, DEBEN responder con el ID a agregar y NO llamar al backend.
- **RF-01.17** Antes de firmar un token, el bot DEBE consultar `GET /users/{uuid}` con un token de rol `user` y vida de 60 s, y rechazar usuarios inexistentes (`404`) o con `is_active = false`. El rol del token final DEBE ser el del backend (normalizado como en el backend), nunca uno configurado.
- **RF-01.18** Los tokens firmados DEBEN tener `{sub, role, exp, jti}`, HS256 y vida `TELEGRAM_TOKEN_TTL_MINUTES` (default 60, rango 1–1440). Al vencer, el bot DEBE re-firmar repitiendo la verificación de RF-01.17.
- **RF-01.19** Si el Telegram ID deja de estar mapeado al mismo usuario, la próxima renovación DEBE terminar la sesión.
- **RF-01.20** Si el backend rechaza el token con `401`, el bot DEBE informar que `JWT_SECRET` no coincide y loguear un `error`.
- **RF-01.21** Al arrancar con `AUTH_METHOD=telegram`, el bot DEBE loguear un `warn` indicando que puede actuar como cualquier usuario del backend, y otro si `JWT_SECRET` es corto o un valor de desarrollo conocido.

```
Given AUTH_METHOD=telegram, JWT_SECRET igual al del backend y TELEGRAM_USERS=123456789:<uuid de default>
When el usuario de Telegram 123456789 envía /start
Then el bot pide GET /users/<uuid> con un token de rol user y 60 s de vida
 And firma un token con el rol real del usuario
 And responde "Hola default 👋 Entraste con tu cuenta de Telegram, sin contraseña."
When el usuario de Telegram 555 envía /start
Then el bot responde que su ID 555 no está en TELEGRAM_USERS, sin llamar al backend
```

```
Given un chat sin sesión y un usuario "default" activo en el backend
When el usuario envía /login, "default" y "ContraseniaSegura2026!"
Then el bot borra el mensaje con la contraseña
 And llama POST /auth/login con {"username":"default","password":"…"}
 And guarda JWT y credenciales cifradas en SQLite
 And responde "Hola default 👋"
When pasan 25 horas (el JWT venció) y el usuario envía "/expense 500 #coffee"
Then el bot re-loguea solo y crea la transacción sin pedir nada
When el bot se reinicia
Then la sesión sigue activa
```

### RF-02 Transacciones

- **RF-02.1 Alta rápida.** `/expense <amount> [date] [#category] [@account] [description]` y `/income ...` DEBEN crear la transacción en un solo mensaje:
  - tipo: `OUT` para `/expense`, `IN` para `/income` (resueltos por `code` vía `GET /transaction-types`);
  - cuenta: la indicada con `@nombre`, si no la cuenta por defecto;
  - categoría: la indicada con `#nombre` (coincidencia sin mayúsculas ni acentos, primero exacta y luego por prefijo único);
  - fecha: ahora, salvo `today`/`hoy`, `yesterday`/`ayer`, `dd/mm` o `dd/mm/yyyy` como primera palabra después del monto;
  - descripción: el resto del texto (opcional, 1–400 caracteres).
- **RF-02.2** Si la categoría falta o es ambigua, el bot DEBE ofrecer un teclado inline con las categorías del usuario (hasta 30, las más usadas primero según `GET /reports/by-category` + "Sin categoría").
- **RF-02.3** Si la categoría `#x` no existe, el bot DEBE ofrecer "Crear categoría x" (como el combobox del frontend).
- **RF-02.4** El monto DEBE ser positivo, con hasta 2 decimales. Formatos aceptados: `1500`, `1500.5`, `1500,50`, `1.500,50`, `1,500.50`. El bot envía siempre `amount` como string con punto decimal.
- **RF-02.5 Alta guiada.** `/new` DEBE iniciar un asistente: tipo → monto → categoría → cuenta → fecha → descripción → confirmación. `/cancel` aborta en cualquier paso.
- **RF-02.6** Tras crear, el bot DEBE mostrar un resumen con botones `✏️ Editar` y `🗑 Borrar`.
- **RF-02.7 Listado.** `/transactions [filters]` DEBE listar transacciones paginadas (10 por página, botones ◀ ▶), ordenadas por fecha descendente (`sort=-transaction_date`, default del backend). Filtros: `@account`, `#category`, `expenses|income`, `month:mm/yyyy`, `from:dd/mm[/yyyy]`, `to:dd/mm[/yyyy]`, texto libre (→ `search`). Mapea a los query params de `GET /transactions`.
- **RF-02.8** Cada fila DEBE mostrar referencia corta, fecha, signo (− gasto, + ingreso), monto con símbolo de moneda, categoría y descripción truncada.
- **RF-02.9 Detalle.** `/show <ref>` DEBE mostrar todos los campos con nombres resueltos (tipo, categoría, cuenta).
- **RF-02.10 Edición.** `/edit <ref>` (o botón) DEBE ofrecer elegir el campo (monto, tipo, categoría, cuenta, fecha, descripción) y enviar `PATCH /transactions/{uuid}` solo con ese campo. Para desvincular categoría se envía `null` explícito. La descripción no se puede vaciar (el backend exige 1–400 caracteres).
- **RF-02.11 Baja.** `/delete <ref>` (o botón) DEBE pedir confirmación inline y luego llamar `DELETE /transactions/{uuid}` (soft delete).

```
Given usuario vinculado, cuenta por defecto "default" (ARS) y categoría "Coffee"
When envía "/expense 1.500,50 #coffee cortado con medialunas"
Then el bot hace POST /transactions con
     {"amount":"1500.50","type_uuid":<OUT>,"category_uuid":<Coffee>,
      "account_uuid":<default>,"transaction_date":<ahora UTC>,
      "description":"cortado con medialunas"}
 And responde "− $ 1.500,50 · Coffee · default" con botones Editar/Borrar
```

### RF-03 Cuentas

- **RF-03.1** `/accounts` DEBE listar las cuentas del usuario con moneda, ⭐ en la por defecto y saldo.
- **RF-03.2** `/account_new <name> <CURRENCY>` DEBE crear la cuenta (`POST /accounts` con `user_uuid` de la sesión). Moneda validada contra `GET /currencies`. Nombre 1–150 caracteres.
- **RF-03.3** `/account_edit <ref>` DEBE permitir cambiar nombre, descripción y moneda.
- **RF-03.4** `/account_default <ref>` DEBE enviar `PATCH /accounts/{uuid}` con `is_default: true`.
- **RF-03.5** `/account_delete <ref>` DEBE pedir confirmación, avisando cuántas transacciones tiene la cuenta, y llamar `DELETE /accounts/{uuid}`.
- **RF-03.6** El bot NO DEBE permitir borrar la única cuenta del usuario, ni la cuenta por defecto sin elegir otra antes.

### RF-04 Categorías

- **RF-04.1** `/categories` DEBE listar las categorías del usuario en orden alfabético.
- **RF-04.2** `/category_new <name>` DEBE crear la categoría (`POST /categories` con `user_uuid` de la sesión). Nombre 1–120 caracteres. Rechazar duplicados (sin mayúsculas ni acentos).
- **RF-04.3** `/category_edit <ref>` DEBE permitir renombrar y cambiar descripción.
- **RF-04.4** `/category_delete <ref>` DEBE pedir confirmación, avisando cuántas transacciones la usan.

### RF-05 Monedas y tipos de transacción

- **RF-05.1** `/currencies` DEBE listar `code`, `name` y `symbol` (todos los roles).
- **RF-05.2** `/types` DEBE listar `name` y `code` (todos los roles).
- **RF-05.3** Solo para `admin` y `manager`: `/currency_new <CODE> <symbol> <name…>`, `/currency_edit <CODE>` (nombre, símbolo, descripción), `/currency_delete <CODE>` (con confirmación). `CODE` = 3 letras, se envía en mayúscula; nombre 1–100; símbolo 1–10.
- **RF-05.4** Solo para `admin` y `manager`: `/type_new <code> <name…>`, `/type_edit <ref>`, `/type_delete <ref>` (con confirmación). `code` 1–50, nombre 1–100.
- **RF-05.5** El bot DEBE impedir borrar o cambiar el `code` de los tipos `IN` y `OUT`, porque `/expense` e `/income` dependen de ellos.
- **RF-05.6** Para otros roles, estos comandos NO aparecen en el menú. Si se escriben igual, el bot responde "No tenés permisos para esa operación." sin llamar a la API.

### RF-06 Reportes

- **RF-06.1** `/balance [@account]` DEBE mostrar, por cuenta, ingresos, gastos y saldo con `GET /reports/balance`, y los totales por moneda. Nunca suma montos de monedas distintas.
- **RF-06.2** `/summary [mm/yyyy] [@account]` DEBE mostrar los gastos del mes por categoría (total y %) con `GET /reports/by-category`, como el gráfico del dashboard del frontend, más el total de ingresos, gastos y neto. Por defecto: mes actual y cuenta por defecto. Los límites del mes se calculan en `BOT_TZ` y se envían en UTC (`from` inclusivo, `to` exclusivo).

### RF-07 Ayuda y UX

- **RF-07.1** `/start` DEBE explicar cómo vincular si no hay vínculo, o mostrar la ayuda si lo hay. `/help` DEBE listar los comandos del rol con ejemplos.
- **RF-07.2** `/cancel` DEBE salir de cualquier diálogo en curso.
- **RF-07.3** Las referencias cortas DEBEN reemplazarse cuando se emite un listado nuevo del mismo recurso. Una referencia inexistente responde "Referencia vencida, volvé a listar". También se acepta el UUID completo.
- **RF-07.4** Los errores de la API DEBEN traducirse a mensajes en español (tabla en `design.md` §8). Nunca mostrar stack traces, JWT ni secretos.

## 6. Requisitos no funcionales

- **RNF-01 Stack**: Rust estable (edición 2021), `tokio`, `teloxide`, `reqwest` (rustls), `sqlx` (SQLite), `serde`, `rust_decimal`, `chrono`/`chrono-tz`, `uuid`.
- **RNF-02 Seguridad**:
  - token del bot, `CREDENTIALS_KEY`, contraseñas y JWT nunca en logs;
  - contraseñas solo cifradas en reposo (AES-256-GCM), nunca en claro ni en el estado de diálogos;
  - archivo SQLite con permisos `0600`;
  - HTTPS hacia el backend fuera de la red local;
  - allowlist opcional;
  - rate limit de login en el bot (RF-01.11).
- **RNF-03 Rendimiento**: respuesta < 2 s para alta rápida con backend local. Catálogos (monedas, tipos, categorías, cuentas) cacheados en memoria por usuario, TTL 5 min, invalidados al modificar.
- **RNF-04 Zona horaria**: `BOT_TZ` (default `America/Argentina/Buenos_Aires`) para interpretar y mostrar fechas. El backend recibe UTC (RFC 3339).
- **RNF-05 Formato numérico**: salida es-AR (`$ 1.500,50`), símbolo según la moneda de la cuenta.
- **RNF-06 Observabilidad**: logs estructurados (`tracing`) con `telegram_user_id`, comando, endpoint, status y latencia. Nunca bodies.
- **RNF-07 Testabilidad**: cliente API testeado contra servidor mock (`wiremock`); parsers y cifrado con tests unitarios; cobertura de parsers ≥ 90 %.
- **RNF-08 Despliegue**: binario único + Dockerfile, configuración por variables de entorno (documentadas en `design.md` §4), con su propio `docker-compose.yml` en `money_manager_telegram` (volumen para el SQLite). No requiere cambios de configuración en el backend.
- **RNF-09 Autoría**: cabecera `// Copyright (C) 2026 Marcos Gabriel Miller` como el resto del repo.

## 7. Matriz comando → endpoint

| Comando | Endpoint(s) |
|---------|-------------|
| `/login` | `POST /auth/login` |
| (automático, re-login) | `POST /auth/login` |
| `/logout` | `POST /auth/logout` |
| `/me` | `GET /users/{uuid}` |
| `/expense`, `/income`, `/new` | `GET /transaction-types`, `GET /categories`, `GET /accounts`, `POST /transactions` (`POST /categories` si se crea categoría) |
| `/transactions` | `GET /transactions?account_uuid&category_uuid&type_uuid&date_from&date_to&search&page&per_page` |
| `/show` | `GET /transactions/{uuid}` |
| `/edit` | `PATCH /transactions/{uuid}` |
| `/delete` | `DELETE /transactions/{uuid}` |
| `/accounts` | `GET /accounts` + `GET /reports/balance` |
| `/account_new` | `GET /currencies`, `POST /accounts` |
| `/account_edit`, `/account_default` | `PATCH /accounts/{uuid}` |
| `/account_delete` | `DELETE /accounts/{uuid}` |
| `/categories` | `GET /categories` |
| `/category_new` | `POST /categories` |
| `/category_edit` | `PATCH /categories/{uuid}` |
| `/category_delete` | `DELETE /categories/{uuid}` |
| `/currencies` | `GET /currencies` |
| `/currency_new`, `/currency_edit`, `/currency_delete` | `POST /currencies`, `PATCH /currencies/{code}`, `DELETE /currencies/{code}` |
| `/types` | `GET /transaction-types` |
| `/type_new`, `/type_edit`, `/type_delete` | `POST /transaction-types`, `PATCH /transaction-types/{uuid}`, `DELETE /transaction-types/{uuid}` |
| `/balance` | `GET /reports/balance?account_uuid` |
| `/summary` | `GET /reports/by-category?from&to&account_uuid` con `type_code=OUT` (gastos) y `type_code=IN` (ingresos) |

## 8. Hallazgos en el backend

Relevados en `money_manager_backend/src`. Todos corregidos y verificados contra la API en vivo (Postgres del `docker-compose.yml`, 29 checks).

| ID | Hallazgo | Corrección | Impacto en el bot |
|----|----------|------------|-------------------|
| BK-1 | `transactions` `get_one`, `create`, `update` y `delete` no llamaban `authorize_action`: cualquier usuario autenticado podía leer, editar o borrar transacciones ajenas por UUID. | El dueño se resuelve vía `accounts.user_uuid`. `create`/`update` validan la cuenta destino y la categoría. `list` también autoriza. | `403` se traduce según `design.md` §8. |
| BK-2 | `update` de transacciones ponía `category_uuid` y `account_uuid` en `NULL` si se omitían. | `Option<Option<Uuid>>`: ausente = no cambia, `null` = desvincula. Test unitario. | El bot hace PATCH parciales. |
| BK-3 | `GET /transactions` no ordenaba. | Param `sort`: `transaction_date`, `amount`, `created_at`, con `-` para descendente. Default `-transaction_date`, desempate por `uuid`. Valor inválido → `400`. | El bot pagina directo contra el backend. |
| BK-4 | No había endpoints de saldo ni agregación. | `GET /reports/balance` (por cuenta y por moneda; `to` opcional) y `GET /reports/by-category` (`from`, `to`, `type_code`, `account_uuid`; total y % por categoría y moneda). Agregación en SQL, mismo alcance por dueño que `GET /transactions`. | `/balance`, `/summary` y `/accounts` usan estos endpoints. |
| BK-5 | `logout` no revocaba el JWT. | Claim `jti` en el JWT; tabla `revoked_tokens`; `logout` requiere token y lo revoca; el extractor rechaza tokens revocados; limpieza de vencidos en cada logout; audit log `LOGOUT`. El frontend no se modificó: su logout no llama al backend, así que sus tokens no se revocan al salir. | `/logout` revoca el JWT del bot. Tras el deploy, los JWT emitidos antes (sin `jti`) dejan de valer: todos deben volver a iniciar sesión una vez. |
| BK-6 | `LoginDto` validaba la política de contraseña en el login (`400` en vez de `401`). | Solo `length(min = 1)`. | — |
| BK-7 | Al borrar cuenta o categoría no se verificaba si tenía transacciones. | Cuenta con transacciones activas → `409 CONFLICT` con la cantidad. Categoría → se desvincula de sus transacciones y se borra, en una transacción de base de datos; el audit log registra cuántas se desvincularon. | `/account_delete` informa el `409`; `/category_delete` avisa que las transacciones quedan "Sin categoría". |
| BK-8 | Con BK-1, un `user` no puede crear ni dejar transacciones sin cuenta (quedarían sin dueño). | Comportamiento esperado del backend. El frontend no se modificó: si se deja la cuenta vacía recibe `403`. | El bot siempre asigna cuenta. |

## 9. Preguntas abiertas

Ninguna.
