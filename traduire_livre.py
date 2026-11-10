#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
traduire_livre.py - Traduit un PDF anglais en français en gardant la mise en page.

Trois étapes, chacune reprenable :
  1. extraire     : lit les blocs de texte du PDF           -> travail/blocs.json
  2. traduire     : envoie les blocs à un LLM local          -> travail/traductions.json
  3. reconstruire : efface l'anglais, écrit le français      -> livre_fr.pdf

Exemples :
  python traduire_livre.py tout livre.pdf --pages 7-8,60 --moteur factice
  python traduire_livre.py tout livre.pdf --pages 7-12 --url http://localhost:1234/v1 --modele qwen3-8b
  python traduire_livre.py tout livre.pdf          (le livre entier)

Dépendance : pip install pymupdf
Usage personnel uniquement : ne diffuse pas la version traduite.
"""
import argparse
import html
import json
import os
import re
import statistics
import sys
import time
import urllib.error
import urllib.request

import pymupdf

DOSSIER_TRAVAIL = "travail"
DOSSIER_POLICES = "polices"
FICHIER_BLOCS = os.path.join(DOSSIER_TRAVAIL, "blocs.json")
FICHIER_TRADUCTIONS = os.path.join(DOSSIER_TRAVAIL, "traductions.json")
FICHIER_GLOSSAIRE = "glossaire.json"

# Marqueurs de style conservés dans le texte envoyé au LLM
BALISES = ("<b>", "</b>", "<i>", "</i>")


# --------------------------------------------------------------------------
# Utilitaires
# --------------------------------------------------------------------------
def journal(message):
    print(time.strftime("[%H:%M:%S]"), message, flush=True)


def lire_json(chemin, defaut):
    if os.path.exists(chemin):
        with open(chemin, encoding="utf-8") as f:
            return json.load(f)
    return defaut


def ecrire_json(chemin, donnees):
    os.makedirs(os.path.dirname(chemin) or ".", exist_ok=True)
    temporaire = chemin + ".tmp"
    with open(temporaire, "w", encoding="utf-8") as f:
        json.dump(donnees, f, ensure_ascii=False, indent=1)
    os.replace(temporaire, chemin)  # écriture atomique : pas de fichier à moitié écrit


def lire_pages(texte, nombre_pages):
    """Transforme '7-9,60' en [7, 8, 9, 60]. Vide = toutes les pages."""
    if not texte:
        return list(range(1, nombre_pages + 1))
    pages = []
    for morceau in texte.split(","):
        morceau = morceau.strip()
        if "-" in morceau:
            debut, fin = morceau.split("-")
            pages.extend(range(int(debut), int(fin) + 1))
        elif morceau:
            pages.append(int(morceau))
    return [p for p in pages if 1 <= p <= nombre_pages]


def sans_balises(texte):
    for balise in BALISES:
        texte = texte.replace(balise, "")
    return texte


def union_rectangles(rectangles):
    return [
        min(r[0] for r in rectangles), min(r[1] for r in rectangles),
        max(r[2] for r in rectangles), max(r[3] for r in rectangles),
    ]


# --------------------------------------------------------------------------
# Étape 1 : extraction
# --------------------------------------------------------------------------
def style_police(nom_police):
    nom = nom_police.lower()
    gras = any(m in nom for m in ("smbd", "bold", "black", "semibold"))
    italique = any(m in nom for m in ("italic", "oblique"))
    return gras, italique


def texte_span(span):
    return "".join(c["c"] for c in span["chars"])


def construire_texte_balise(bloc):
    """Texte du bloc avec <b>/<i>, césures de fin de ligne recollées."""
    jetons = []  # [texte, gras, italique, fin_de_ligne]
    for ligne in bloc["lines"]:
        for span in ligne["spans"]:
            gras, italique = style_police(span["font"])
            jetons.append([texte_span(span), gras, italique, False])
        if jetons:
            jetons[-1][3] = True

    # Recollage des césures : "transla-" + "tion" -> "translation"
    for i, jeton in enumerate(jetons):
        if jeton[3]:
            suivant = jetons[i + 1][0].lstrip() if i + 1 < len(jetons) else ""
            if jeton[0].rstrip().endswith("-") and suivant[:1].islower():
                jeton[0] = jeton[0].rstrip()[:-1]
            elif not jeton[0].endswith(" "):
                jeton[0] += " "

    # Regroupement des jetons de même style, balises placées autour du texte seul
    resultat = ""
    i = 0
    while i < len(jetons):
        style = (jetons[i][1], jetons[i][2])
        morceau = jetons[i][0]
        i += 1
        while i < len(jetons) and (jetons[i][1], jetons[i][2]) == style:
            morceau += jetons[i][0]
            i += 1
        coeur = morceau.strip()
        if not coeur:
            resultat += morceau
            continue
        avant = morceau[: len(morceau) - len(morceau.lstrip())]
        apres = morceau[len(morceau.rstrip()):]
        ouvre = ("<b>" if style[0] else "") + ("<i>" if style[1] else "")
        ferme = ("</i>" if style[1] else "") + ("</b>" if style[0] else "")
        resultat += avant + ouvre + coeur + ferme + apres
    return re.sub(r"\s+", " ", resultat).strip()


def propriete_dominante(bloc):
    """Police, taille et couleur du span le plus long (le plus représentatif)."""
    meilleur = max(
        (s for l in bloc["lines"] for s in l["spans"]),
        key=lambda s: len(s["chars"]),
    )
    return meilleur["font"], round(meilleur["size"], 1), meilleur["color"]


def reconstruire_titre_espace(blocs_titre):
    """
    Les gros titres sont écrits lettre par lettre ('M a k i n g  f i r e').
    On remesure les écarts entre lettres : un écart nettement plus grand que
    l'écart habituel marque une coupure de mot.
    """
    lettres = []
    for b in blocs_titre:
        for ligne in b["lines"]:
            for span in ligne["spans"]:
                for c in span["chars"]:
                    if c["c"].strip():
                        lettres.append((c["bbox"][0], c["bbox"][2], c["c"]))
    lettres.sort()
    if len(lettres) < 2:
        return "".join(l[2] for l in lettres)
    ecarts = [lettres[i + 1][0] - lettres[i][1] for i in range(len(lettres) - 1)]
    mediane = statistics.median(ecarts)
    taille = blocs_titre[0]["lines"][0]["spans"][0]["size"]
    seuil = max(mediane * 1.9, mediane + 0.3 * taille)
    texte = lettres[0][2]
    for i in range(1, len(lettres)):
        if ecarts[i - 1] > seuil:
            texte += " "
        texte += lettres[i][2]
    return texte


def extraire_page(page):
    brut = page.get_text("rawdict")
    taille_page = [page.rect.width, page.rect.height]
    candidats, deja_vus = [], set()
    for bloc in brut["blocks"]:
        if bloc["type"] != 0 or not bloc["lines"]:
            continue
        # Texte pivoté : ignoré pour l'instant
        if any(abs(l["dir"][1]) > 0.5 for l in bloc["lines"]):
            continue
        texte = construire_texte_balise(bloc)
        cle = (tuple(round(x) for x in bloc["bbox"]), texte)
        if cle in deja_vus:  # certains blocs sont dupliqués dans le PDF
            continue
        deja_vus.add(cle)
        police, taille, couleur = propriete_dominante(bloc)
        candidats.append({"brut": bloc, "texte": texte, "police": police,
                          "taille": taille, "couleur": couleur})

    longs = [c["taille"] for c in candidats if len(sans_balises(c["texte"])) >= 60]
    taille_corps = statistics.median(longs) if longs else 9.0

    resultat, titres = [], []
    for c in candidats:
        brut = c["brut"]
        lettres = re.sub(r"[^A-Za-zÀ-ÿ]", "", sans_balises(c["texte"]))
        base = {"bbox": [round(x, 2) for x in brut["bbox"]], "texte": c["texte"],
                "police": c["police"], "taille": c["taille"], "couleur": c["couleur"],
                "nb_lignes": len(brut["lines"])}
        if len(lettres) == 1 and c["taille"] >= 3 * taille_corps:
            base["type"] = "lettrine"
        elif "castellar" in c["police"].lower() and c["taille"] >= 14 and lettres:
            titres.append((c, base))
            continue
        elif len(lettres) < 2 or re.fullmatch(r"[\d\s]+", sans_balises(c["texte"])):
            base["type"] = "ignorer"  # numéros de page, numéros de légende...
        else:
            base["type"] = "texte"
        resultat.append(base)

    # Titres espacés : regroupés par ligne puis reconstruits
    titres.sort(key=lambda t: (t[0]["brut"]["bbox"][1], t[0]["brut"]["bbox"][0]))
    groupes = []
    for c, base in titres:
        centre = (c["brut"]["bbox"][1] + c["brut"]["bbox"][3]) / 2
        if groupes and abs(centre - groupes[-1]["centre"]) < 0.7 * c["taille"]:
            groupes[-1]["membres"].append((c, base))
        else:
            groupes.append({"centre": centre, "membres": [(c, base)]})
    for g in groupes:
        membres = g["membres"]
        zones = [m[1]["bbox"] for m in membres]
        resultat.append({
            "type": "titre", "bbox": union_rectangles(zones), "zones": zones,
            "texte": reconstruire_titre_espace([m[0]["brut"] for m in membres]),
            "police": membres[0][0]["police"], "taille": membres[0][0]["taille"],
            "couleur": membres[0][0]["couleur"], "nb_lignes": 1,
        })

    # Lettrines : rattachées au bloc de texte voisin qui commence en minuscule
    for lettrine in [b for b in resultat if b["type"] == "lettrine"]:
        lx0, ly0, lx1, ly1 = lettrine["bbox"]
        voisins = [
            b for b in resultat if b["type"] == "texte"
            and lx0 - 5 <= b["bbox"][0] <= lx1 + 80
            and ly0 - 40 <= b["bbox"][1] <= ly1 + 20
            and sans_balises(b["texte"])[:1].islower()
        ]
        if voisins:
            cible = min(voisins, key=lambda b: abs(b["bbox"][0] - lx1) + abs(b["bbox"][1] - ly0))
            cible["zones"] = cible.get("zones", [cible["bbox"]]) + [lettrine["bbox"]]
            cible["texte"] = lettrine["texte"].strip() + cible["texte"]
            cible["bbox"] = union_rectangles([cible["bbox"], lettrine["bbox"]])
            lettrine["type"] = "ignorer"
        else:
            lettrine["type"] = "ignorer"

    for i, b in enumerate(resultat):
        b["id"] = i
        b.setdefault("zones", [b["bbox"]])
    return {"taille_page": taille_page, "taille_corps": taille_corps, "blocs": resultat}


def etape_extraire(args):
    doc = pymupdf.open(args.pdf)
    pages = lire_pages(args.pages, len(doc))
    donnees = lire_json(FICHIER_BLOCS, {"pdf": args.pdf, "pages": {}})
    donnees["pdf"] = args.pdf
    for numero in pages:
        donnees["pages"][str(numero)] = extraire_page(doc[numero - 1])
        nb = sum(1 for b in donnees["pages"][str(numero)]["blocs"] if b["type"] in ("texte", "titre"))
        journal(f"Extraction page {numero}/{len(doc)} : {nb} blocs à traduire")
    ecrire_json(FICHIER_BLOCS, donnees)


# --------------------------------------------------------------------------
# Étape 2 : traduction
# --------------------------------------------------------------------------
PROMPT_SYSTEME = """Tu es un traducteur professionnel anglais -> français.
Tu traduis un guide illustré, au ton direct et un peu humoristique, sur la reconstruction d'une civilisation (feu, eau, médecine, plantes, navigation, industrie...).

Règles strictes :
- Traduis fidèlement, en français naturel et fluide, sans résumer ni ajouter.
- Le texte est découpé en blocs numérotés [[1]], [[2]]... Réponds avec les MÊMES marqueurs, dans le même ordre, un bloc par marqueur.
- Conserve exactement les balises <b>, </b>, <i>, </i> autour des mots correspondants.
- Conserve les nombres, les unités et les noms propres. Convertis rien.
- Ne rajoute aucun commentaire, aucune explication, aucune note.
- Les titres courts restent courts."""


def charger_glossaire():
    return lire_json(FICHIER_GLOSSAIRE, {})


def glossaire_utile(glossaire, texte):
    bas = texte.lower()
    lignes = [f"- {en} -> {fr}" for en, fr in glossaire.items() if en.lower() in bas]
    return "Glossaire obligatoire :\n" + "\n".join(lignes) if lignes else ""


def nettoyer_reponse(texte):
    texte = re.sub(r"<think>.*?</think>", "", texte, flags=re.S)
    return texte.strip()


def appeler_llm(args, messages):
    corps = json.dumps({
        "model": args.modele, "messages": messages,
        "temperature": 0.2, "max_tokens": 4096, "stream": False,
    }).encode("utf-8")
    requete = urllib.request.Request(
        args.url.rstrip("/") + "/chat/completions", data=corps,
        headers={"Content-Type": "application/json", "Authorization": "Bearer local"})
    with urllib.request.urlopen(requete, timeout=args.delai) as reponse:
        donnees = json.loads(reponse.read().decode("utf-8"))
    return nettoyer_reponse(donnees["choices"][0]["message"]["content"])


def traduire_factice(texte):
    """Faux français pour tester la mise en page : le texte rallongé de 18 %."""
    rallonge = texte + " " + " ".join(texte.split()[: max(1, int(len(texte.split()) * 0.18))])
    return rallonge


def traduire_lot(args, textes, glossaire):
    """Traduit une liste de textes ; renvoie une liste de même longueur (None = échec)."""
    if args.moteur == "factice":
        return [traduire_factice(t) for t in textes]

    suffixe = "\n/no_think" if args.sans_reflexion else ""
    envoi = "\n".join(f"[[{i + 1}]] {t}" for i, t in enumerate(textes))
    systeme = PROMPT_SYSTEME
    gl = glossaire_utile(glossaire, envoi)
    if gl:
        systeme += "\n\n" + gl
    messages = [{"role": "system", "content": systeme},
                {"role": "user", "content": envoi + suffixe}]
    for tentative in range(1, args.essais + 1):
        try:
            reponse = appeler_llm(args, messages)
        except (urllib.error.URLError, TimeoutError, ConnectionError) as erreur:
            journal(f"  Erreur réseau (tentative {tentative}/{args.essais}) : {erreur}")
            time.sleep(3 * tentative)
            continue
        morceaux = re.split(r"\[\[(\d+)\]\]", reponse)
        trouves = {int(morceaux[i]): morceaux[i + 1].strip() for i in range(1, len(morceaux) - 1, 2)}
        if len(textes) == 1 and not trouves:
            trouves = {1: reponse}  # le modèle a oublié le marqueur sur un bloc unique
        resultat = [trouves.get(i + 1) for i in range(len(textes))]
        if all(r for r in resultat):
            return resultat
        journal(f"  Réponse incomplète ({sum(1 for r in resultat if r)}/{len(textes)}), nouvel essai")
    return [None] * len(textes)


def rapport_taille_plausible(source, traduction):
    ls, lt = len(sans_balises(source)), len(sans_balises(traduction))
    return ls == 0 or 0.4 <= lt / ls <= 3.0


def etape_traduire(args):
    donnees = lire_json(FICHIER_BLOCS, None)
    if not donnees:
        sys.exit("blocs.json introuvable : lance d'abord l'étape 'extraire'.")
    glossaire = charger_glossaire()
    # cache : texte anglais exact -> texte français (garantit la cohérence des répétitions)
    cache = lire_json(FICHIER_TRADUCTIONS, {})
    pages = lire_pages(args.pages, max(int(p) for p in donnees["pages"]))
    for numero in pages:
        page = donnees["pages"].get(str(numero))
        if not page:
            continue
        a_faire = []
        for bloc in page["blocs"]:
            if bloc["type"] in ("texte", "titre") and bloc["texte"] not in cache and bloc["texte"] not in a_faire:
                a_faire.append(bloc["texte"])
        if not a_faire:
            journal(f"Page {numero} : déjà traduite")
            continue

        # Lots de 6000 caractères maximum, pour rester fiable avec un modèle 8B
        lots, courant, taille = [], [], 0
        for t in a_faire:
            if courant and taille + len(t) > 6000:
                lots.append(courant)
                courant, taille = [], 0
            courant.append(t)
            taille += len(t)
        if courant:
            lots.append(courant)

        journal(f"Traduction page {numero} : {len(a_faire)} blocs en {len(lots)} lot(s)")
        for lot in lots:
            resultats = traduire_lot(args, lot, glossaire)
            if None in resultats:  # repli : bloc par bloc
                resultats = [r or (traduire_lot(args, [t], glossaire)[0]) for r, t in zip(resultats, lot)]
            for source, fr in zip(lot, resultats):
                if fr and rapport_taille_plausible(source, fr):
                    cache[source] = fr
                else:
                    journal(f"  ATTENTION : échec ou longueur suspecte, texte laissé en anglais : {source[:50]!r}")
        ecrire_json(FICHIER_TRADUCTIONS, cache)  # sauvegarde après chaque page = reprise possible


# --------------------------------------------------------------------------
# Étape 3 : reconstruction
# --------------------------------------------------------------------------
def css_polices():
    return (
        "@font-face{font-family:crimson;src:url(CrimsonText-Regular.ttf);}"
        "@font-face{font-family:crimson;font-weight:bold;src:url(CrimsonText-Bold.ttf);}"
        "@font-face{font-family:crimson;font-style:italic;src:url(CrimsonText-Italic.ttf);}"
        "@font-face{font-family:cinzel;src:url(Cinzel-Regular.ttf);}"
        "@font-face{font-family:cinzel;font-weight:bold;src:url(Cinzel-Regular.ttf);}"
        "body,div,p{margin:0;padding:0;}"
    )


def vers_html(texte):
    """Échappe &, < et > mais garde nos balises de style."""
    for i, balise in enumerate(BALISES):
        texte = texte.replace(balise, f"\x00{i}\x00")
    texte = html.escape(texte, quote=False)
    for i, balise in enumerate(BALISES):
        texte = texte.replace(f"\x00{i}\x00", balise)
    return texte


def rectangle_etendu(bloc, tous_les_blocs, hauteur_page):
    """Étend le cadre vers le bas jusqu'au prochain bloc de la même colonne."""
    x0, y0, x1, y1 = bloc["bbox"]
    limite = hauteur_page - 30
    for autre in tous_les_blocs:
        if autre is bloc or autre["type"] not in ("texte", "titre"):
            continue
        ax0, ay0, ax1, ay1 = autre["bbox"]
        recouvrement = min(x1, ax1) - max(x0, ax0)
        if recouvrement > 0.3 * (x1 - x0) and ay0 >= y1 - 1:
            limite = min(limite, ay0 - 3)
    hauteur = y1 - y0
    y1_etendu = min(limite, y1 + 0.4 * hauteur) if bloc["type"] == "texte" else y1
    return pymupdf.Rect(x0 - 0.5, y0 - 0.5, x1 + 1.5, max(y1, y1_etendu) + 0.5)


def construire_html_bloc(bloc, traduction):
    police = bloc["police"].lower()
    titre = "castellar" in police
    famille = "cinzel" if titre else "crimson"
    couleur = "#%06x" % bloc["couleur"]
    taille = bloc["taille"]
    if bloc["nb_lignes"] > 1:
        hauteur = bloc["bbox"][3] - bloc["bbox"][1]
        interligne = min(1.5, max(1.0, hauteur / bloc["nb_lignes"] / taille))
    else:
        interligne = 1.1
    espacement = "letter-spacing:0.5pt;" if titre else ""
    style = (f"font-family:{famille};font-size:{taille}pt;line-height:{interligne:.2f};"
             f"color:{couleur};text-align:left;{espacement}")
    return f'<div style="{style}">{vers_html(traduction)}</div>'


def etape_reconstruire(args):
    donnees = lire_json(FICHIER_BLOCS, None)
    cache = lire_json(FICHIER_TRADUCTIONS, {})
    if not donnees:
        sys.exit("blocs.json introuvable : lance d'abord l'étape 'extraire'.")
    doc = pymupdf.open(args.pdf)
    pages = lire_pages(args.pages, len(doc))
    archive = pymupdf.Archive(DOSSIER_POLICES)
    css = css_polices()
    nb_reduits, nb_echecs = 0, 0

    for numero in pages:
        page = doc[numero - 1]
        infos = donnees["pages"].get(str(numero))
        if not infos:
            continue
        blocs = [b for b in infos["blocs"] if b["type"] in ("texte", "titre") and b["texte"] in cache]
        # 1) effacer le texte anglais (sans rien peindre : l'illustration dessous reste visible)
        for b in blocs:
            for zone in b["zones"]:
                page.add_redact_annot(pymupdf.Rect(zone) + (0.3, 0.3, -0.3, -0.3), fill=False)
        page.apply_redactions(images=pymupdf.PDF_REDACT_IMAGE_NONE,
                              graphics=pymupdf.PDF_REDACT_LINE_ART_NONE)
        # 2) écrire le français, en réduisant la police si ça déborde
        for b in blocs:
            rect = rectangle_etendu(b, infos["blocs"], infos["taille_page"][1])
            contenu = construire_html_bloc(b, cache[b["texte"]])
            reste, echelle = page.insert_htmlbox(rect, contenu, css=css, archive=archive, scale_low=0.5)
            if reste < 0:  # ne rentre pas même réduit à 50 % : on laisse déborder
                page.insert_htmlbox(rect + (0, 0, 0, 40), contenu, css=css, archive=archive, scale_low=0)
                nb_echecs += 1
                journal(f"  Page {numero} : bloc trop long, débordement toléré ({cache[b['texte']][:40]!r})")
            elif echelle < 0.999:
                nb_reduits += 1
        journal(f"Reconstruction page {numero} : {len(blocs)} blocs")

    if args.pages:
        doc.select([p - 1 for p in pages])  # on ne garde que les pages demandées
    doc.save(args.sortie, garbage=4, deflate=True)
    journal(f"Terminé : {args.sortie} ({nb_reduits} blocs réduits, {nb_echecs} débordements)")


# --------------------------------------------------------------------------
# Ligne de commande
# --------------------------------------------------------------------------
def main():
    analyseur = argparse.ArgumentParser(description="Traduit un PDF anglais -> français en gardant la mise en page")
    analyseur.add_argument("etape", choices=["extraire", "traduire", "reconstruire", "tout"])
    analyseur.add_argument("pdf", help="PDF source")
    analyseur.add_argument("--pages", default="", help="ex. 7-9,60 (vide = tout le livre)")
    analyseur.add_argument("--moteur", choices=["local", "factice"], default="local",
                           help="'factice' = faux français pour tester la mise en page")
    analyseur.add_argument("--url", default="http://localhost:1234/v1",
                           help="API compatible OpenAI (LM Studio : :1234/v1, Ollama : :11434/v1)")
    analyseur.add_argument("--modele", default="qwen3-8b", help="nom exact du modèle chargé")
    analyseur.add_argument("--sans-reflexion", action="store_true", default=True,
                           help="ajoute /no_think pour Qwen3 (plus rapide)")
    analyseur.add_argument("--essais", type=int, default=3)
    analyseur.add_argument("--delai", type=int, default=600, help="délai maximal par requête (secondes)")
    analyseur.add_argument("--sortie", default="livre_fr.pdf")
    args = analyseur.parse_args()

    if args.etape in ("extraire", "tout"):
        etape_extraire(args)
    if args.etape in ("traduire", "tout"):
        etape_traduire(args)
    if args.etape in ("reconstruire", "tout"):
        etape_reconstruire(args)


if __name__ == "__main__":
    main()
