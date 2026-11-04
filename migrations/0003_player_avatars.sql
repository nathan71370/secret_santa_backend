-- Photo de profil tenue par limperiam-auth : chemin relatif (`/avatars/3.jpg`)
-- recopié à chaque connexion et à chaque lecture de l'annuaire. NULL : pas de photo.
ALTER TABLE players ADD COLUMN avatar_path TEXT;
