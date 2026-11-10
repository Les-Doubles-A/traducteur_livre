# Traducteur de livres (Rust)

Dépôt GitHub : **https://github.com/Les-Doubles-A/traducteur_livre**

Application de bureau pour traduire des livres entre n'importe quelles langues.

## Lancer

```
cargo build --release
target\release\traducteur_livre.exe
```

1. Glisser un ou plusieurs livres dans la fenêtre.
2. Choisir les langues (source en détection auto possible, "Autre..." pour une langue absente de la liste).
3. Choisir le format de sortie, puis **Traduire**.

Le fichier traduit est créé à côté de l'original : `livre.fr.epub`.

## Formats

| Entrée | Sortie |
|---|---|
| EPUB, PDF (texte), DOCX, ODT, FB2, HTML, MD, TXT | EPUB, PDF, DOCX, HTML, MD, TXT |

Option **Bilingue** : le texte original apparaît sous chaque paragraphe.

## Moteurs

- **IA (OpenAI compatible)** : LM Studio (`http://localhost:1234/v1`), Ollama (`http://localhost:11434/v1`), OpenAI, OpenRouter...
- **DeepL** : clé API (gratuite ou pro).
- **Test** : aucune traduction, pour vérifier les formats.

## Bon à savoir

- Le travail est mis en cache (`%LOCALAPPDATA%\traducteur_livre`) : après une coupure ou une erreur, relancer reprend où ça s'était arrêté.
- `glossaire.json` impose des traductions de termes (`{"source": "cible"}`).
- PDF en sortie : police Crimson Text intégrée pour l'alphabet latin, police système (Arial, SimHei...) pour les autres. Pour l'arabe ou l'hébreu, préférer EPUB/DOCX/HTML (pas de mise en forme RTL dans le PDF).
- Un PDF en entrée est converti en texte continu : la mise en page d'origine n'est pas conservée (le script Python `traduire_livre.py` reste là pour ça). Un PDF scanné nécessite d'abord une OCR.
- Tests : `cargo test` (et `cargo test live -- --ignored` avec LM Studio lancé).

<!-- installeur-release -->
## Installation

Téléchargez **`traducteur_livre-Setup-0.1.0.exe`** depuis la [dernière release](https://github.com/Les-Doubles-A/traducteur_livre/releases/latest), puis lancez-le.

L'installation se fait pour l'utilisateur courant, sans droits administrateur (par défaut dans `%LOCALAPPDATA%\Programs\traducteur_livre`), avec un raccourci dans le menu Démarrer et une désinstallation depuis les Paramètres Windows.

L'installeur n'est pas signé : si Windows SmartScreen l'affiche, cliquez sur « Informations complémentaires » puis « Exécuter quand même ».

### Recréer l'installeur

Compiler l'application (`cargo build --release`), puis lancer [Inno Setup 6](https://jrsoftware.org/isinfo.php) : `iscc installer\traducteur_livre.iss`. Le setup est créé dans `installer\Output\`.
