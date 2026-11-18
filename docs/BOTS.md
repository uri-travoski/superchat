# Bots, webhooks, forms and scheduled messages

Admins create bots in the admin panel (Bots page). A bot is an account with a bearer token; it can only post into
conversations an admin added it to (people can also start a direct message with it). Messages from bots show an **APP** tag.
Below, `$S` is the server address and `$T` a bot token (`bt_...`, shown in the admin panel).

## Telegram-style API

The same shape as `api.telegram.org`: `POST $S/bot$T/<method>` with JSON, form or query parameters; replies are `{"ok":true,"result":...}`.
`chat_id` is the conversation id (see `getChat`/`getUpdates`).

```sh
curl $S/bot$T/getMe
curl $S/bot$T/sendMessage -d chat_id=$CONV -d text='Deploy *finished*' -d parse_mode=Markdown
curl $S/bot$T/sendDocument -F chat_id=$CONV -F document=@receipt.pdf -F caption='Receipt'
curl $S/bot$T/getUpdates -d offset=0 -d timeout=0        # polling; acknowledge with offset = last update_id + 1
curl $S/bot$T/setWebhook -d url=https://example.com/hook -d secret_token=s3cret   # pushes updates instead of polling
curl $S/bot$T/deleteWebhook
```

Supported methods: `getMe`, `getChat`, `sendMessage`, `sendDocument`, `sendPhoto`, `editMessageText`, `deleteMessage`, `getUpdates`,
`setWebhook`, `deleteWebhook`, `getWebhookInfo`, `getFile` (download at `$S/file/bot$T/<path>`), `sendChatAction` and `answerCallbackQuery` (accepted, no-op).
A webhook delivery carries `X-Telegram-Bot-Api-Secret-Token` when a secret was set.

## Native REST API

Bearer auth: `Authorization: Bearer $T`.

```sh
curl -H "Authorization: Bearer $T" $S/api/v1/bot/me
curl -H "Authorization: Bearer $T" $S/api/v1/bot/conversations
curl -H "Authorization: Bearer $T" -H 'Content-Type: application/json' $S/api/v1/bot/messages \
  -d '{"conv_id":"'$CONV'","body":"Hello **team**","client_id":"once-1"}'     # client_id makes retries idempotent
curl -H "Authorization: Bearer $T" "$S/api/v1/bot/updates?offset=0"
```

Also `PATCH|DELETE /bot/messages/{id}`, `POST /bot/files`, `GET /bot/files/{id}`, `GET|PUT|DELETE /bot/webhook`, `GET /bot/conversations/{id}/messages`.

## Forms (SV-17)

Attach a `form` to a message. Clients draw it as a card; a button with its own `form` opens a modal. Field types: `text`, `multiline`,
`boolean`, `select`, `multi_select`.

```sh
curl -H "Authorization: Bearer $T" -H 'Content-Type: application/json' $S/api/v1/bot/messages -d '{
  "conv_id": "'$CONV'", "body": "Approve the purchase?",
  "form": {"id":"approve","title":"Purchase #42","fields":[
      {"id":"note","type":"text","label":"Note","required":false},
      {"id":"urgent","type":"boolean","label":"Urgent"}],
    "buttons":[{"id":"yes","label":"Approve","style":"primary"},{"id":"no","label":"Reject","style":"danger"}],
    "submit_label":"Send"}}'
```

The response arrives as an update (`form.submitted`: message id, user, button id, values) through `getUpdates`/`/bot/updates`, the bot webhook,
or an outgoing webhook subscribed to `form.submitted`. Users answer with `POST /api/v1/messages/{id}/form-response {button_id, values}`.

## App Home

A bot can publish a home tab (`GET|PUT /api/v1/bot/home`, blocks of text, buttons and forms). Clients show it on the bot's Home tab.

## Incoming webhooks

Admin panel, Webhooks page: creates a secret URL `$S/api/v1/hooks/<id>/<secret>` that posts into one conversation.

```sh
curl -H 'Content-Type: application/json' $S/api/v1/hooks/$ID/$SECRET -d '{"text":"Build 123 passed"}'
```

## Outgoing webhooks

Events (`message.created`, `message.updated`, `message.deleted`, `form.submitted`) are POSTed as JSON with
`X-SuperChat-Event`, `X-SuperChat-Delivery`, `X-SuperChat-Timestamp` and `X-SuperChat-Signature: sha256=<hex>` (the 1.0 names `X-Atomic-*` are still sent with the same values) = HMAC-SHA256 of
`"<timestamp>.<body>"` with the webhook secret. Failures are retried with growing delays and survive a server restart.
Reject deliveries with an old timestamp to stop replays.

## Scheduled messages

`POST /api/v1/conversations/{id}/scheduled {body, due_ms}`; `GET /scheduled` lists yours; `DELETE /scheduled/{id}` cancels.
They are stored in the database, so they are sent after a restart (late ones are sent on start-up).

## AI (optional, off by default)

Admin panel, AI page: provider (Anthropic or OpenAI-compatible), model, base URL and key. **When on, the text a user searches
for, and the messages a user asks to be recapped, are sent to that provider.** Clients show this notice. Endpoints: `POST /ai/search`, `POST /ai/recap`.

## Example: receipt logger and AI agent (Python, standard library only)

```python
import json, os, time, urllib.request, urllib.parse

S, T = os.environ["ATOMIC_SERVER"], os.environ["BOT_TOKEN"]

def call(method, **params):
    data = urllib.parse.urlencode(params).encode()
    with urllib.request.urlopen(f"{S}/bot{T}/{method}", data) as r:
        return json.load(r)["result"]

def ask_model(text):          # replace with your AI provider call
    return "Logged: " + text[:80]

offset = 0
while True:
    for u in call("getUpdates", offset=offset, timeout=0):
        offset = u["update_id"] + 1
        m = u.get("message")
        if not m or "text" not in m:
            continue
        with open("receipts.log", "a") as f:       # receipt logger
            f.write(json.dumps(m) + "\n")
        call("sendMessage", chat_id=m["chat"]["id"], text=ask_model(m["text"]))   # AI agent reply
    time.sleep(2)
```
