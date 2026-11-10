import json
import re

MOTS_ANGLAIS = set("the of and to is in that it with for as are this be on or you your can use from by an at will have not which water make when if but they more also into all one has".split())

with open("travail/traductions.json", encoding="utf-8") as f:
    cache = json.load(f)

def est_anglais(texte):
    mots = re.findall(r"[a-z']+", texte.lower())
    return len(mots) >= 6 and sum(m in MOTS_ANGLAIS for m in mots) / len(mots) > 0.18

a_refaire = [cle for cle, valeur in cache.items()
             if valeur.strip() == cle.strip() or est_anglais(valeur)]
print(len(a_refaire), "entrées supprimées du cache")
for cle in a_refaire:
    del cache[cle]

with open("travail/traductions.json", "w", encoding="utf-8") as f:
    json.dump(cache, f, ensure_ascii=False, indent=1)
    