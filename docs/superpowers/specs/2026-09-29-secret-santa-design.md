# Secret Santa — design

Réécriture de `secret_santa_v2` (Java, fichier `players.txt`, lancé à la main
depuis IntelliJ) en application web déployée sur l'infra Komodo :

- **backend** : `backend/secret_santa_rs/` — Rust, SQLite, stack Komodo `secret_santa_backend`
- **frontend** : `frontend/secret-santa/` — Vue 3, servi par nginx, stack Komodo `secret_santa_frontend`
- **SSO** : `frontend/limperiam-auth/` — une route ajoutée, une app déclarée

Accessible sur `https://santa.limperiam.com`, uniquement via le SSO limperiam.

## Objectifs

1. Les joueurs s'inscrivent eux-mêmes (« je participe » / « je ne participe
   pas ») avant une deadline. Pas de réponse = ne participe pas.
2. À la deadline, le tirage se lance tout seul et chaque participant reçoit sa
   cible par mail, à l'adresse de son compte SSO.
3. Un joueur retrouve sa cible sur le site, voit les autres participants (jamais
   qui offre à qui) et leur historique de cadeaux reçus.
4. L'admin (`isAdmin` du SSO) crée l'édition, règle budget et deadline, garde la
   main sur les inscriptions et suit l'envoi des mails — sans jamais voir les
   paires.
5. L'historique des tirages Java (`players.txt`) est importable, pour que la
   contrainte « pas la même cible que l'an dernier » continue de s'appliquer.

## Hors périmètre

- Exclusions personnalisées (couples, etc.) : seule la contrainte « pas la même
  cible que l'édition précédente » existe, comme dans le Java.
- Liste de souhaits, messagerie anonyme entre donneur et receveur.
- Révélation des donneurs après Noël.
- Plusieurs éditions simultanées : une seule édition par année civile.

## Architecture

```
navigateur ──https──▶ Traefik (santa.limperiam.com)
                        ├─ PathPrefix(/api) ─▶ secret-santa-backend:8080  (axum)
                        │                          ├─▶ limperiam-auth:3000 (réseau traefik)
                        │                          ├─▶ SMTP
                        │                          └─▶ /data (SQLite + photos)
                        └─ le reste ─────────▶ secret-santa-frontend:80  (nginx, dist/ Vue)
```

Un seul domaine, routage par chemin : le navigateur ne voit qu'une origine, le
cookie `lim_session` part tout seul vers `/api`, aucun CORS. Le routeur backend
a une priorité Traefik supérieure à celle du frontend.

Deux stacks Komodo indépendantes, chacune dans son dépôt git, chacune sur le
réseau externe `traefik`. Même conventions que `limperiam-auth` : pas de
`container_name`, pas de `ports:`, `tls=true` sans `certresolver`, variables
obligatoires en `${VAR:?message}`, volume nommé pour `/data`.

## Authentification

Chaque requête `/api/*` (sauf `/api/health`) passe par un extracteur axum
`CurrentUser` :

1. Lit le cookie `lim_session`. Absent → **401**.
2. `GET http://limperiam-auth:3000/api/session` en relayant ce cookie. Réponse
   mise en cache 60 s, clé = hash SHA-256 du jeton (jamais le jeton en clair en
   mémoire de cache ni dans les logs).
3. Erreur réseau, 5xx, réponse illisible → **401** (échec fermé).
4. `santa` absent de `apps` → **403**.
5. Sinon `CurrentUser { user_id, email, pseudo, is_admin }`. Upsert de la ligne
   `players` correspondante (email en minuscules, pseudo à jour).

L'extracteur `AdminUser` ajoute : `is_admin == false` → **403**.

Les routes qui modifient l'état (`POST`/`PUT`/`PATCH`/`DELETE`) vérifient en plus
que l'en-tête `Origin` vaut `PUBLIC_ORIGIN` (`https://santa.limperiam.com`),
sinon **403** — le cookie de domaine est envoyé par tout sous-domaine, c'est la
défense CSRF.

**Mode développement** : `AUTH_DEV_USER=email@x.fr,pseudo,admin` court-circuite
le SSO. Le serveur **refuse de démarrer** si cette variable est définie sans
`APP_ENV=development`.

### Modification de limperiam-auth

Ajout de `GET /api/admin/users?app=<slug>` dans
`src/app/api/admin/users/route.ts` :

- `requireAdmin()` (session admin obligatoire, sinon 403).
- Renvoie `{ users: [{ userId, email, pseudo }] }` : comptes **non désactivés**
  ayant accès à l'app `slug` (même règle que `allowedApps`). Sans `app`, tous
  les comptes non désactivés.
- Tests dans le style existant du dépôt.

Le backend Santa l'appelle en relayant le cookie de l'admin (route
`GET /api/admin/sso-users`). Aucun secret partagé : seul un admin connecté peut
déclencher la lecture des adresses.

L'app `santa` est à déclarer dans le catalogue depuis `/admin` du SSO (URL
`https://santa.limperiam.com`), ouverte au groupe `potes`.

## Données (SQLite, migrations sqlx embarquées)

```sql
players (
  id            INTEGER PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE,     -- minuscules, clé de rapprochement
  sso_user_id   INTEGER UNIQUE,           -- NULL pour un joueur importé jamais vu
  pseudo        TEXT NOT NULL,
  has_access    INTEGER NOT NULL DEFAULT 0, -- dans le dernier instantané SSO ?app=santa
  updated_at    TEXT NOT NULL
)

editions (
  id            INTEGER PRIMARY KEY,
  year          INTEGER NOT NULL UNIQUE,
  budget_eur    INTEGER NOT NULL CHECK (budget_eur > 0),
  deadline      TEXT NOT NULL,            -- UTC ISO-8601
  status        TEXT NOT NULL CHECK (status IN ('open','drawn','failed')),
  failure_reason TEXT,
  drawn_at      TEXT,
  reminder_sent_at TEXT,
  announced_at  TEXT,
  imported      INTEGER NOT NULL DEFAULT 0, -- édition issue de players.txt
  created_at    TEXT NOT NULL
)

registrations (
  edition_id    INTEGER NOT NULL REFERENCES editions(id) ON DELETE CASCADE,
  player_id     INTEGER NOT NULL REFERENCES players(id),
  participating INTEGER NOT NULL,         -- 1 oui, 0 non ; absence = pas répondu
  answered_by_admin INTEGER NOT NULL DEFAULT 0,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (edition_id, player_id)
)

assignments (
  edition_id    INTEGER NOT NULL REFERENCES editions(id) ON DELETE CASCADE,
  giver_id      INTEGER NOT NULL REFERENCES players(id),
  receiver_id   INTEGER NOT NULL REFERENCES players(id),
  mail_status   TEXT NOT NULL CHECK (mail_status IN ('pending','sent','failed','none')),
  mail_error    TEXT,
  PRIMARY KEY (edition_id, giver_id),
  UNIQUE (edition_id, receiver_id)
)

gifts (
  id            INTEGER PRIMARY KEY,
  player_id     INTEGER NOT NULL REFERENCES players(id),  -- le receveur
  year          INTEGER NOT NULL,
  description   TEXT NOT NULL,            -- 1..500 caractères
  photo_path    TEXT,                     -- nom de fichier dans /data/uploads
  created_at    TEXT NOT NULL
)
```

`mail_status = 'none'` pour les éditions importées (aucun mail n'est parti).

## Cycle de vie d'une édition

```
[admin crée] ──▶ open ──(deadline passée ou « Tirer maintenant »)──▶ drawn
                  ▲                     │
                  │                     └─ tirage impossible ──▶ failed
                  └──── admin repousse la deadline ◀──────────────────┘
```

- **Création** (admin) : année = année courante (unique), budget, deadline
  future. Le backend rafraîchit l'instantané SSO (`?app=santa`, avec le cookie
  admin) puis passe l'édition en `open` et envoie le mail d'ouverture à tous les
  joueurs `has_access = 1` (tâche de fond, `announced_at` renseigné à la fin).
- **open** : chaque joueur répond oui/non et peut changer d'avis jusqu'à la
  deadline. L'admin peut répondre à la place de quelqu'un
  (`answered_by_admin = 1`), modifier budget et deadline.
- **Relance** : 48 h avant la deadline, mail aux joueurs `has_access = 1` sans
  ligne dans `registrations`. Une seule fois (`reminder_sent_at`). Si la
  deadline est fixée à moins de 48 h, pas de relance.
- **Tirage** : voir ci-dessous. Succès → `drawn`, les inscriptions sont figées.
- **failed** : l'admin reçoit un mail avec la raison. Il repousse la deadline
  (→ `open`, `failure_reason` effacé) ou déclenche « Tirer maintenant ».
- **Annuler le tirage** (admin, `drawn` → `open` avec nouvelle deadline) : efface
  les `assignments` de l'édition. Dernier recours, double confirmation dans l'UI.
  Les mails déjà partis ne sont pas rattrapés : l'UI le dit explicitement.

## Tirage

Module pur `draw` (aucune I/O), testé unitairement :

```rust
fn draw(participants: &[PlayerId], previous: &HashMap<PlayerId, PlayerId>, rng: &mut impl Rng)
    -> Result<Vec<(PlayerId, PlayerId)>, DrawError>
```

- Moins de 3 participants → `DrawError::NotEnoughParticipants`.
- Même algorithme que le Java : mélange, cycle unique (`i → i+1`), rejet si un
  donneur retombe sur sa cible de l'édition précédente, 1000 essais max →
  `DrawError::NoValidDraw`.
- « Édition précédente » = l'édition `drawn` de plus grande année strictement
  inférieure (importée ou non).

Orchestration (`draw_service`) :

1. `BEGIN IMMEDIATE`. `UPDATE editions SET status='drawn', drawn_at=… WHERE id=?
   AND status IN ('open','failed')` — si 0 ligne, quelqu'un d'autre a tiré :
   on s'arrête sans erreur. C'est ce qui rend le tirage idempotent face à la
   tâche de fond et au bouton « Tirer maintenant » simultanés.
2. Lit les participants (`participating = 1`) et les cibles précédentes, appelle
   `draw`. En cas d'erreur : `status='failed'`, `failure_reason`, `COMMIT`, mail
   à l'admin (`ADMIN_EMAIL`).
3. Insère les `assignments` (`mail_status='pending'`), `COMMIT`.
4. Envoie les mails de résultat en tâche de fond, un par seconde (comme le
   Java), en mettant à jour `mail_status` / `mail_error` au fil de l'eau.

Au démarrage, les `assignments` restées `pending` (crash pendant l'envoi) sont
renvoyées.

### Tâche de fond

Une boucle tokio, toutes les 60 s :
- éditions `open` dont `deadline <= now` → tirage ;
- éditions `open` à moins de 48 h de la deadline, sans `reminder_sent_at` →
  relance.

L'état vit en base : si le conteneur était arrêté à la deadline, le tirage part
au premier tour de boucle après le démarrage.

## Mails (lettre, SMTP)

Tous en texte + HTML simple, en français, expéditeur `SMTP_FROM`.

| Mail | Destinataires | Contenu |
|---|---|---|
| Ouverture | `has_access = 1` | Année, budget, deadline (heure de Paris), lien |
| Relance | `has_access = 1` sans réponse | Deadline, lien |
| Résultat | chaque donneur | Pseudo de la cible, budget, lien vers son profil |
| Échec du tirage | `ADMIN_EMAIL` | Raison, lien vers l'admin |

L'admin peut renvoyer le mail de résultat d'un donneur (`failed` ou non). Rien
n'est journalisé qui contienne une adresse ou une cible.

## Import de `players.txt`

`POST /api/admin/import` (multipart : fichier + année). Format Java :
`nom;email;cible` — la cible est le **nom** d'un autre joueur du même fichier.

- Année déjà existante → 409.
- Chaque ligne : rapprochement par email (minuscules) avec `players` ; créé
  (`sso_user_id = NULL`, pseudo = nom) s'il n'existe pas. Il sera relié à son
  compte SSO à sa première connexion ou au prochain instantané.
- Cible résolue par nom (insensible à la casse) dans le fichier.
- Crée une édition `drawn`, `imported = 1`, budget saisi (défaut 30), avec ses
  `assignments` (`mail_status='none'`), dans une seule transaction.
- Réponse : nombre de paires importées + lignes ignorées avec la raison (ligne
  mal formée, cible introuvable, cible en double). Tout ou rien si le fichier
  ne donne aucune paire valide.

## API

Toutes les réponses en JSON, erreurs `{ "error": "CODE", "message": "texte FR" }`.

**Public**
- `GET /api/health` → 200 (healthcheck Docker, sans auth)

**Joueur connecté**
- `GET /api/me` → `{ playerId, pseudo, email, isAdmin }`
- `GET /api/editions/current` → édition de l'année courante (ou `null`) :
  année, budget, deadline, statut, participants (`playerId`, `pseudo`), ma
  réponse (`yes`/`no`/`null`)
- `PUT /api/editions/current/registration` `{ participating: bool }` — 409 si
  l'édition n'est pas `open` ou deadline passée
- `GET /api/editions/current/my-target` → `{ playerId, pseudo }`, 404 si pas de
  tirage ou si je ne participe pas
- `GET /api/players` → joueurs ayant au moins une participation ou un cadeau
  (`playerId`, `pseudo`)
- `GET /api/players/:id` → `{ pseudo, gifts: [{ id, year, description, photoUrl }] }`
- `POST /api/me/gifts` (multipart : `year`, `description`, `photo?`)
- `PUT /api/me/gifts/:id` (multipart, mêmes champs, `removePhoto?`)
- `DELETE /api/me/gifts/:id`
- `GET /api/uploads/:file` → photo (nom aléatoire, en-têtes `Cache-Control` long)

Les routes `/me/gifts/:id` renvoient 404 (pas 403) si le cadeau appartient à
quelqu'un d'autre.

Photos : JPEG/PNG/WebP (vérifié sur les octets, pas l'extension), 5 Mo max,
redimensionnées à 1600 px max et réencodées (retire les métadonnées EXIF,
dont la géolocalisation), stockées sous un nom aléatoire.

**Admin**
- `GET /api/admin/sso-users` → rafraîchit l'instantané SSO et renvoie chaque
  joueur `has_access` avec sa réponse pour l'édition courante
- `POST /api/admin/editions` `{ budgetEur, deadline }`
- `PATCH /api/admin/editions/current` `{ budgetEur?, deadline? }` — une nouvelle
  deadline sur une édition `failed` la repasse en `open`
- `PUT /api/admin/editions/current/registrations/:playerId` `{ participating: bool | null }`
- `POST /api/admin/editions/current/draw` → tirer maintenant
- `DELETE /api/admin/editions/current/draw` `{ deadline }` → annuler le tirage
- `GET /api/admin/editions/current/mails` → `[{ playerId, pseudo, mailStatus, mailError }]`
  (jamais le receveur)
- `POST /api/admin/editions/current/mails/:playerId/resend`
- `POST /api/admin/import`

## Frontend (`frontend/secret-santa/`)

Vue 3 + Vite + TypeScript, Vue Router, Pinia pour la session. Design system en
CSS maison (variables, mode sombre), direction visuelle définie au moment de
l'implémentation avec le skill frontend-design. Tous les appels partent en
relatif vers `/api`.

**Démarrage** : `GET /api/me`. 401 → redirection vers
`https://auth.limperiam.com/login?next=<url courante>`. 403 → page « pas
d'accès ».

**Écrans**
1. **Accueil**
   - `open` : compte à rebours jusqu'à la deadline, boutons « Je participe » /
     « Je ne participe pas » (état courant mis en avant), budget, participants
     inscrits.
   - `drawn` : carte à retourner « Tu offres un cadeau à… » → pseudo et lien
     vers le profil de la cible. Non-participant : message adapté.
   - `failed` / aucune édition : message d'attente.
2. **Participants** : grille d'avatars (initiales, couleur dérivée du pseudo),
   chacun vers son profil.
3. **Profil** : pseudo, cadeaux reçus groupés par année (description + photo,
   visionneuse au clic). Sur son propre profil : ajouter / modifier / supprimer.
4. **Admin** (route gardée par `isAdmin`, masquée sinon)
   - Pas d'édition cette année : formulaire budget + deadline.
   - `open` : compte à rebours, tableau des comptes SSO (pseudo, email,
     réponse, badge « répondu par l'admin »), bascule oui / non / sans réponse,
     modifier budget / deadline, « Tirer maintenant » (confirmation).
   - `failed` : raison, repousser la deadline, « Tirer maintenant ».
   - `drawn` : statut des mails par joueur (envoyé / en attente / échec +
     « Renvoyer »), « Annuler le tirage » (double confirmation, rappel que les
     mails déjà envoyés ne sont pas rattrapés).
   - Import `players.txt` : fichier + année + budget, rapport affiché.

**Dev local** : proxy Vite `/api` → `http://localhost:8080`.

## Configuration du backend

| Variable | Obligatoire | Défaut | Rôle |
|---|---|---|---|
| `DATABASE_PATH` | non | `/data/santa.db` | Fichier SQLite |
| `UPLOADS_DIR` | non | `/data/uploads` | Photos |
| `AUTH_URL` | non | `http://limperiam-auth:3000` | SSO, réseau Docker interne |
| `AUTH_LOGIN_URL` | non | `https://auth.limperiam.com/login` | Renvoyé au front |
| `PUBLIC_ORIGIN` | non | `https://santa.limperiam.com` | Contrôle `Origin` + liens des mails |
| `ADMIN_EMAIL` | **oui** | — | Destinataire des mails d'échec |
| `SMTP_HOST` | **oui** | — | Refus de démarrer sans |
| `SMTP_PORT` | non | `587` | `465` = TLS implicite, sinon STARTTLS |
| `SMTP_USER` / `SMTP_PASS` | non | — | |
| `SMTP_FROM` | **oui** | — | Expéditeur autorisé par le fournisseur SMTP |
| `APP_ENV` | non | `production` | `development` autorise `AUTH_DEV_USER` et journalise les mails au lieu de les envoyer |
| `AUTH_DEV_USER` | non | — | `email,pseudo,admin` — dev uniquement |
| `TZ` | non | `Europe/Paris` | Affichage des dates dans les mails |

Les dates sont stockées en UTC ; le front affiche en heure locale du
navigateur, les mails en `Europe/Paris`.

## Déploiement

**Backend** : Dockerfile multi-stage (`rust:*-slim` → `debian:*-slim` ou
distroless, utilisateur non-root), binaire unique, migrations au démarrage.
`compose.yaml` : service `secret-santa-backend`, réseau `traefik`, volume
`secret-santa-data:/data`, labels Traefik
`Host(santa.limperiam.com) && PathPrefix(/api)` avec `priority=100`,
healthcheck sur `/api/health`.

**Frontend** : Dockerfile multi-stage (`node` → `nginx:alpine`), `try_files
$uri /index.html`, cache long sur `/assets/*`, pas de cache sur `index.html`.
`compose.yaml` : service `secret-santa-frontend`, réseau `traefik`, labels
`Host(santa.limperiam.com)`, priorité par défaut.

**Ordre de mise en service**
1. Déployer `limperiam-auth` avec la nouvelle route, déclarer l'app `santa`.
2. Déployer le backend, puis le frontend.
3. Importer `players.txt` depuis l'admin.
4. Créer l'édition de l'année.

## Tests

- **Rust, unitaires** : `draw` (moins de 3, contrainte respectée, cas
  insoluble, distribution sur beaucoup de tirages : jamais d'auto-attribution,
  un seul cycle) ; parseur `players.txt` ; validation des entrées.
- **Rust, intégration** : routeur axum sur SQLite en mémoire, SSO simulé
  (trait `AuthClient`), SMTP simulé (trait `Mailer` qui enregistre les envois).
  Couvre : 401/403, CSRF `Origin`, inscription avant/après deadline, tirage
  idempotent (deux déclenchements concurrents), `failed` puis repousser, la
  cible visible uniquement par son donneur, la route mails admin sans
  receveur, import (succès, lignes ignorées, année existante), cadeaux d'autrui
  → 404, tâche de fond (tirage + relance unique) avec horloge injectée.
- **limperiam-auth** : tests de la nouvelle route (non-admin 403, filtre
  `app`, comptes désactivés exclus).
- **Front** : `vue-tsc` + build, Vitest sur le compte à rebours et le client
  API (redirection 401).
