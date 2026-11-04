# secret_santa_rs

Backend du Secret Santa de `santa.limperiam.com` : inscriptions avec deadline,
tirage automatique, mails de résultat, profils et historique des cadeaux.
Remplace `secret_santa_v2` (Java, fichier `players.txt`).

Design : [`docs/superpowers/specs/2026-09-29-secret-santa-design.md`](docs/superpowers/specs/2026-09-29-secret-santa-design.md)
Frontend : `frontend/secret-santa` (stack Komodo séparée, même domaine).

## Développement local

```bash
APP_ENV=development \
ADMIN_EMAIL=ton@email.fr \
AUTH_DEV_USER='ton@email.fr,Nathan,admin' \
DATABASE_PATH=./data/santa.db UPLOADS_DIR=./data/uploads \
BIND_ADDR=127.0.0.1:8080 PUBLIC_ORIGIN=http://localhost:5173 MAIL_DELAY_MS=0 \
cargo run
```

`AUTH_DEV_USER` (`email,pseudo[,admin]`) court-circuite le SSO ; le serveur
refuse de démarrer s'il est défini sans `APP_ENV=development`. En
développement, les mails sont écrits dans les logs au lieu de partir.
`PUBLIC_ORIGIN` doit valoir l'origine du front (Vite : `http://localhost:5173`),
sinon les écritures sont refusées (contrôle `Origin`).

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## Variables d'environnement

| Variable | Obligatoire | Défaut | Rôle |
|---|---|---|---|
| `ADMIN_EMAIL` | **oui** | — | Reçoit le mail d'échec du tirage automatique |
| `SMTP_HOST` | **oui** (hors dev) | — | Serveur SMTP |
| `SMTP_FROM` | **oui** avec `SMTP_HOST` | — | Expéditeur autorisé par le fournisseur |
| `SMTP_PORT` | non | `587` | `465` = TLS implicite, sinon STARTTLS |
| `SMTP_USER` / `SMTP_PASS` | non | — | |
| `AUTH_URL` | non | `http://limperiam-auth:3000` | SSO, par le réseau Docker |
| `AUTH_LOGIN_URL` | non | `https://auth.limperiam.com/login` | Renvoyé au front sur 401 |
| `AUTH_PUBLIC_URL` | non | `https://auth.limperiam.com` | Origine publique du SSO : préfixe des photos de profil (`avatarUrl`), chargées directement par le navigateur avec le cookie SSO |
| `PUBLIC_ORIGIN` | non | `https://santa.limperiam.com` | Contrôle `Origin`, liens des mails |
| `DATABASE_PATH` | non | `/data/santa.db` | |
| `UPLOADS_DIR` | non | `/data/uploads` | Photos des cadeaux |
| `BIND_ADDR` | non | `0.0.0.0:8080` | |
| `MAIL_DELAY_MS` | non | `1000` | Pause entre deux mails d'une vague |
| `APP_ENV` | non | `production` | `development` : mails journalisés, `AUTH_DEV_USER` permis |
| `AUTH_DEV_USER` | non | — | Dev uniquement |
| `RUST_LOG` | non | `info` | |
| `TZ` | non | `Europe/Paris` (compose) | Horodatage des logs seulement : les dates des mails sont toujours affichées à l'heure de Paris |
| `SANTA_HOST` | non | `santa.limperiam.com` | `compose.yaml` uniquement : hôte de la règle Traefik (pas lu par le binaire) |

## Mise en service

1. Déployer `limperiam-auth` avec la route `GET /api/admin/users`, puis dans
   `https://auth.limperiam.com/admin` créer l'application `santa`
   (URL `https://santa.limperiam.com`), ouverte aux groupes `potes` et `admin`.
2. Komodo : Stack `secret_santa_backend` sur ce dépôt (`compose.yaml`), avec
   `ADMIN_EMAIL`, `SMTP_HOST`, `SMTP_FROM` (+ `SMTP_USER`/`SMTP_PASS`).
   Déployer ; vérifier `https://santa.limperiam.com/api/health` → `ok`.
3. Déployer le Stack `secret_santa_frontend`.
4. Dans l'admin du site : importer le `players.txt` de l'an dernier.
5. Créer l'édition de l'année (budget + deadline) : l'annonce part à tous.

## Fonctionnement

- **Tirage automatique** : une boucle vérifie chaque minute les éditions dont
  la deadline est passée. L'état est en base : si le conteneur était arrêté à
  la deadline, le tirage part au redémarrage. La transition vers `drawn` est
  conditionnelle : impossible de tirer deux fois.
- **Relance** : 48 h avant la deadline, une fois, aux comptes sans réponse.
- **Échec** (moins de 3 inscrits, contrainte de l'an dernier insoluble) :
  l'édition passe en `failed`, l'admin reçoit un mail ; repousser la deadline
  la rouvre.
- **Mails** : un par seconde, en tâche de fond. Un arrêt en pleine vague laisse
  des envois `pending`, renvoyés au démarrage suivant.
- **Secret** : aucune route ne révèle les paires, admin compris ; chacun ne
  voit que sa propre cible.
- **Sauvegarde** : tout est dans le volume `secret-santa-data` (`santa.db` +
  `uploads/`).
