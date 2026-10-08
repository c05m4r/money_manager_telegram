# money_manager_telegram

Bot de Telegram en Rust para cargar y consultar tus finanzas personales en
[money_manager_backend](https://github.com/c05m4r/money_manager_backend): gastos, ingresos,
cuentas, categorías, monedas, tipos de transacción, saldos y resúmenes mensuales.

El bot es un cliente HTTP de la API. El backend no sabe nada de Telegram. La especificación
completa (SDD) está en [`specs/`](specs/).

## Cómo funciona la sesión

1. Enviás `/login`, tu usuario o email y tu contraseña del backend. El bot **borra el mensaje con la contraseña** apenas lo lee.
2. El bot llama `POST /auth/login` y guarda el JWT en su SQLite.
3. Con `STORE_CREDENTIALS=true` (default) también guarda tus credenciales **cifradas** (AES-256-GCM, clave `CREDENTIALS_KEY`). Cuando el JWT vence (24 h), el bot vuelve a loguearse solo.
4. `/logout` revoca el JWT en el backend y borra todo lo guardado.

Si cambiás la contraseña en el backend, el próximo re-login falla y el bot te pide `/login` otra vez.

## Requisitos

- Rust 1.85+ (edición 2024) o Docker.
- `money_manager_backend` corriendo y accesible.
- Un bot creado con [@BotFather](https://t.me/BotFather) (`/newbot` → token).

## Configuración

```bash
cp env/.env.example env/.env
openssl rand -base64 32   # pegalo en CREDENTIALS_KEY
```

| Variable | Obligatoria | Default | Descripción |
|----------|-------------|---------|-------------|
| `TELOXIDE_TOKEN` | sí | — | token de @BotFather |
| `BACKEND_API_URL` | no | `http://127.0.0.1:8000/api/v1` | URL base de la API |
| `ALLOWED_TELEGRAM_IDS` | no | vacío = todos | IDs de Telegram separados por coma. Tu ID te lo da [@userinfobot](https://t.me/userinfobot) |
| `STORE_CREDENTIALS` | no | `true` | guardar credenciales cifradas para re-login automático |
| `CREDENTIALS_KEY` | si `STORE_CREDENTIALS=true` | — | 32 bytes en base64 (`openssl rand -base64 32`) |
| `DATABASE_PATH` | no | `data/bot.sqlite` | SQLite de sesiones y diálogos (se crea con permisos `0600`) |
| `BOT_TZ` | no | `America/Argentina/Buenos_Aires` | zona horaria IANA |
| `CACHE_TTL_SECS` | no | `300` | caché de cuentas, categorías, monedas y tipos |
| `PAGE_SIZE` | no | `10` | filas por página en `/transactions` |
| `HTTP_TIMEOUT_SECS` | no | `10` | timeout de cada request al backend |
| `RUST_LOG` | no | `info` | nivel de logs |

## Ejecutar

```bash
cargo run --release
```

Con Docker (el backend en el host debe escuchar en `0.0.0.0`, y `BACKEND_API_URL=http://host.docker.internal:8000/api/v1`):

```bash
docker compose up -d --build
```

## Comandos

El menú de Telegram muestra solo los comandos de tu rol.

| Comando | Ejemplo |
|---------|---------|
| `/login`, `/logout`, `/me` | `/login default` |
| `/expense` | `/expense 1.500,50 #cafe cortado`, `/expense 8000 ayer #super @visa` |
| `/income` | `/income 250000 #sueldo` |
| `/new` | asistente paso a paso |
| `/transactions` | `/transactions @visa #cafe expenses month:10/2026 from:01/10 to:15/10 texto` |
| `/show`, `/edit`, `/delete` | `/show 3` (número del último `/transactions`) |
| `/accounts` | cuentas con saldo |
| `/account_new`, `/account_edit`, `/account_default`, `/account_delete` | `/account_new Visa USD`, `/account_default 2` |
| `/categories` | categorías |
| `/category_new`, `/category_edit`, `/category_delete` | `/category_new Mascotas` |
| `/currencies`, `/types` | listados |
| `/currency_new`, `/currency_edit`, `/currency_delete` (admin, manager) | `/currency_new BRL R$ Real brasileño` |
| `/type_new`, `/type_edit`, `/type_delete` (admin, manager) | `/type_new transfer Transferencia` |
| `/balance` | `/balance @visa` |
| `/summary` | `/summary 09/2026 @visa` |
| `/cancel`, `/help` | |

Montos: `1500`, `1500,50`, `1.500,50`, `1,500.50`. Fechas: `hoy`, `ayer`, `dd/mm`, `dd/mm/yyyy`.

## Seguridad

- Activá la **verificación en dos pasos de Telegram**: quien controle tu cuenta de Telegram controla tus finanzas en el bot.
- `CREDENTIALS_KEY` y `TELOXIDE_TOKEN` van solo en el entorno del proceso. Nunca en el repo ni en la imagen.
- Quien tenga el archivo SQLite **y** `CREDENTIALS_KEY` puede recuperar contraseñas. Protegé el host del bot como protegés el backend, o usá `STORE_CREDENTIALS=false`.
- Fuera de tu red, usá `https://` en `BACKEND_API_URL` (el bot avisa al arrancar si no).
- Los logs nunca incluyen contraseñas, JWT ni bodies.

## Tests

```bash
cargo test                                   # unitarios + wiremock
cargo clippy --all-targets -- -D warnings

# contra un backend real (crea y borra una transacción de prueba)
MM_LIVE_URL=http://127.0.0.1:8000/api/v1 MM_LIVE_USER=default MM_LIVE_PASSWORD='…' \
  cargo test live -- --ignored
```

### Checklist manual (E2E con Telegram)

1. `/start` → pide `/login`. `/login default` + contraseña → el mensaje de la contraseña desaparece y saluda.
2. `/expense 1500 #cafe prueba` → crea y muestra botones Editar/Borrar.
3. `/expense 1500` (sin categoría) → teclado de categorías → elegir → crea.
4. `/expense 500 #inexistente` → ofrece crear la categoría.
5. `/new` → recorre tipo, monto, categoría, cuenta, fecha, descripción, confirmación.
6. `/transactions` → paginación ◀ ▶; `/show 1`, `/edit 1` (monto y categoría), `/delete 1`.
7. `/accounts`, `/account_new Prueba USD`, `/account_default`, `/account_delete` (rechaza la cuenta por defecto).
8. `/balance`, `/summary`.
9. Reiniciar el bot → la sesión sigue.
10. `/logout` → `/accounts` pide `/login`.

## Licencia

GPL-3.0. Copyright (C) 2026 Marcos Gabriel Miller.
