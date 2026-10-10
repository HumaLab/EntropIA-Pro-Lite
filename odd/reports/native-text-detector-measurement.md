# Native-text bibliography detector — threshold measurement (B2)

Date: 2026-10-08. Detector version: **1** (`BIBLIOGRAPHY_DETECTOR_VERSION`).
Plan: `odd/plans/plan-texto-nativo-parte-b.md` 2.2 ("Validación antes de mergear"),
`odd/plans/plan-texto-nativo.md` 3.4. Task: `odd/tasks/native-text-part-b.md` B2.

## How the measurement ran

- Source database: the `prueba-sync` dev-profile archive
  (`%APPDATA%\com.entropia.shared\dev-profiles\prueba-sync\entropia.sqlite`, WAL, open in the
  owner's app). It was **never opened for writing**: the measurement ran on a copy made with the
  SQLite online backup API (`sqlite3 "file:…?mode=ro" ".backup …"`), and the `-wal`/`-shm` files
  of the original were left in place.
- Copy: `G:\EntropIA-Stack\agent-scratch\prueba-sync-copy.sqlite` (940 MB, `PRAGMA
  integrity_check` = ok). Opened read-only by the test.
- Harness: `apps/desktop/src-tauri/tests/bibliography_detector_measurement.rs`, the `#[ignore]`
  test `measure_bibliography_detector_on_db_copy`, driven by `ENTROPIA_MEASURE_DB` (path to the
  copy). The Zotero data directory comes from the copy's own `app_settings` with the app key
  `ZOTERO_DATA_DIR_SETTING_KEY`; files resolve through the app's own resolver
  (`attachment_ref_for` → `resolve_attachment_file`); page texts come from the part-A PDFium
  reader (`read_pdfium_page_texts`, one instance per 20 pages).
- Scope: every PDF attachment with stored `bibliographic_page_texts` rows. For each stored-`rich`
  page the detector ran on the raw PDFium string (the reader's own output, soft-hyphen marks
  included). For every stored page row the detector ran on the stored text — that count sizes the
  reprocess candidate list.
- The detector saw only text; nothing was written anywhere but this report and the console log.

## Detector v1 (what was measured)

Two independent rules over eligible tokens (whitespace split, edge punctuation stripped, URL /
DOI / email / domain tokens dropped):

1. **Glued words.** Needs ≥ 80 Latin letters (ASCII, Latin-1 Supplement minus `×`/`÷`, Latin
   Extended-A/B). Flags when ≥ 50 % of those letters lie in tokens whose longest Latin-letter run
   is longer than 24 characters.
2. **Old OCR noise.** Needs ≥ 40 eligible tokens of 4+ Latin letters. Flags when ≥ 8 % of them
   contain `.`, `~` or `·` between two letters. Hyphens and apostrophes never count; the Catalan
   geminate `l·l` is excluded; dotted abbreviations (`U.S.`, `U.S.A.`, `N.A.T.O.`, `e.g.`,
   `i.e.`) are excluded.

## Numbers

| | |
|---|---|
| PDF attachments with stored page rows scanned | **587** |
| attachments whose file could not be read | 0 |
| stored page rows seen | 19,606 |
| stored-`rich` pages with a PDFium read | 18,997 |

### Detector on the PDFium text of stored-`rich` pages

| | |
|---|---|
| flagged by rule 1 (glued words) | **0** |
| flagged by rule 2 (old OCR noise) | **421** (2.22 % of scanned) |
| flagged by either rule | 421 |
| distinct attachments with a flagged page | **4** |

Flagged PDFium pages per attachment:

| attachment | flagged pages |
|---|---|
| `b7635184-220d-4887-88a0-76a27b8eab57` (Abulafia 1950, *El problema del yute*, scanned) | 418 |
| `6f80900a-5f63-41d1-b839-3fc855c24516` | 1 |
| `77cbd5ad-759b-4f69-b41c-51bef444aca6` | 1 |
| `820a66bc-e7b5-4a9b-a3a0-822c508be7ed` | 1 |

### Detector on the STORED page texts (reprocess candidate list sizing)

| | |
|---|---|
| flagged by rule 1 (glued words) | **1,479** |
| flagged by rule 2 (old OCR noise) | **753** |
| flagged by either rule | **1,610** (8.21 % of stored rows; 622 by both rules) |
| of those, stored quality `rich` | 1,610 (all of them) |
| distinct attachments with a flagged stored page | **82** |

The plan's own pre-measurement (plan-texto-nativo §1) counted 2,693 damaged `rich` pages in the
same **82 attachments**; detector v1 flags 1,610 of those pages (it needs 50 % of the letters
glued, so partially-spaced rows fall under it). The attachment set matches.

## False-positive criterion (< 0.5 % of clean pages)

- **Rule 1 on PDFium text flags nothing** — 0 of 18,997 pages. Once the words have spaces, no
  page reaches the glued-words threshold.
- **Rule 2 flags 421 pages in 4 attachments.** The random sample of 30 (below) is uniformly
  damaged old-OCR text — those are true positives (the pages are what "repair what is already
  stored" is about). The 418-page attachment is the scanned 1950 book whose OCR layer is
  visibly noisy page after page.
- **The three singleton flags are the only suspected false positives**, all on data/code pages
  where dotted identifiers (e.g. R's `read.csv`) look like noise to rule 2 (shown in full below,
  they are the only flagged pages of their attachments).
- Upper bound if all three are false positives: **3 / 18,576 clean pages ≈ 0.016 %**.

**Criterion met** (< 0.5 %). No threshold was changed. If the owner wants the three data/code
hits gone, the two candidate refinements — both proposals, deliberately not applied — are:

- exclude a noise occurrence when the token carries code markers (`(`, `=`, `_`) or a dotted
  identifier shape (segment after the dot starts lowercase and the token has more than one
  dot-letter pair, like `read.csv` / `data.frame`); or
- raise rule 2's ratio from 8 % to 10 % (would cost some genuinely noisy pages in the 1950 book,
  where single pages sit near the line).

## 30 randomly sampled flagged PDFium pages

Deterministic reservoir sample (seeded xorshift, reproducible). First 200 characters only.

1. `b7635184-220d-4887-88a0-76a27b8eab57` p. 188 — rule 2: Dentro de este t6pico S8 suele hablar en demasía? de la incrementaci6n de la producci6n yutera brasileña , pe r o no debe oLví.dar-se que el centro pr-Lnc í.paI de plantas una vez f'Lrial í.aado e
2. `b7635184-220d-4887-88a0-76a27b8eab57` p. 1393 — rule 2: ln....rcUlbl0 la, cant,ldi1d. d. 70.G",;j (:;etenta mil) tone. ,a4~r1caa (pe.o, llit1#O) de a..rp111~ra óe las 'lspec~.(lcaclon.it, eCl la proporc1&nttu.para cada una de ellu ~8 1nd1cau el anexo N°
3. `b7635184-220d-4887-88a0-76a27b8eab57` p. 1141 — rule 2: -1- -:;,nero 19.34/n1c1ernhre 1941 - -en dólar nortearEricano pOI' I ,) j u.n1dadea 1934 1"('\5 ., t '.' 1~¡,,.,9 11,00 lJ,óU l'{),j~ ". 1';9 1 ,06 1935 lL,J2 L),29 10,20 10 25 '1 .: ')9 l' 29 g:S(
4. `b7635184-220d-4887-88a0-76a27b8eab57` p. 104 — rule 2: Ji ~ien el yut~ ~~ ~ultiv& en diver~c~ ~a!ses con buenos resultados, e] principal p~is pro¿uctor nica, rIJe por -.2.:'.vcr3[.s razones de orden e conéru co-csoc í.sL, 68 e:10~lC:LtrE- en c onc í c
5. `b7635184-220d-4887-88a0-76a27b8eab57` p. 18 — rule 2: c'.irect~¡rr""nte con 1<1.: polítiya pz-op í.c í.ada por el ,jupe- " t ...........í.n» "s oc í a.Lmen .....1,_,.. .4 .. t e:; J·uw·t~,tt ''''.'''''~.). P~ra Go~dyuv~r la re~li e;;.;t~ tr;;. b,;.jo.
6. `b7635184-220d-4887-88a0-76a27b8eab57` p. 272 — rule 2: ,Asu Vez los r-econr'eccí.onador-es <.feb~lsas 'cuentan • ..... r'~" ".. • .... > " • • . • . .J.. ".. . , •• .;. .. J •. • • , ....',. . • .• .• . • .• . ~. .. -' . como mater-aa prim.a con las bo.
7. `b7635184-220d-4887-88a0-76a27b8eab57` p. 20 — rule 2: ti.o· w..yor nUL:H~rQ deelci:;t");ntos puedan s ope aar-se y Luego de s e ch...r 1 0. eL .... e opí,o de ~rpiller. de yute o yute en rilme¡. --por el'c~r"ctt':!'\ perece.dero del producto. - p",rá.t
8. `b7635184-220d-4887-88a0-76a27b8eab57` p. 241 — rule 2: t \ i .!:' ,'~,'P.J"O, . , Las .t~.rniiriadC?ras 'muy s~mejftntes' 'atas ~anteriores' ." • .. - . ... . .f , ,; ~.'o. .' -.' + ' • .. . llevan püas enn:funero, di~111et:r-o. Ylargod~stiIito.lÜ.- pro
9. `b7635184-220d-4887-88a0-76a27b8eab57` p. 181 — rule 2: semilla de yute, dado que sólo se había distribt1.ído 2. ."'-'23 kí.Logr-amos , co r-r-í.éndo s e el r-i e sgo Q, le per~ ., d-2 .. i d d a 8_._1 poder ger-mí.n at.Lvo del r'emanent e que Ll.egaba a
10. `b7635184-220d-4887-88a0-76a27b8eab57` p. 1418 — rule 2: -xv,. Ruta. el 'ld.~ro de 19'0 ~opc1&1de1 Go bernador ceneral de la .Ind1adoa1atiT,de :U recepct6n ele, part.e o c1e la tot.aUdad ,cte' la' &rmtlGad de ,trf..P _no1qu.adaen lA.ol4uaul. Xln. ,en e~J
11. `b7635184-220d-4887-88a0-76a27b8eab57` p. 902 — rule 2: Cuadro N° 34-I,;:.anufacturas de yute: Impor-t.ac í.one a de los;;};. U:J. - 1905/l9l¡.4 (Gont.) Año Arpi- Coberturao Coberturas Arpillera l'Jf,anufac- Cuerdas Galen- lleras aoll$h~ Hilados de Arpi-
12. `b7635184-220d-4887-88a0-76a27b8eab57` p. 481 — rule 2: El f",ctor rr e con el yute "t', '" 11i.... S ',~.ii, estos I~OL)entos no s(~ ha utiliziid.·•. como oucedAnt:'o cJ.elY1,.:rte ¡ el ir:cre;lé~rto de La.s fitr~::-;in.dí&'ma.s por una IMrte )7 1...
13. `b7635184-220d-4887-88a0-76a27b8eab57` p. 189 — rule 2: la cJ.ifusión e sté. en el l:jst~ad.o (le Arnaaorias , d.onde po siblernente las conddc í.one s ecoLóg í cas vsean rnás apro piadas que las nuestras, la luana de obra sea más ba rata y donde pudiera s
14. `b7635184-220d-4887-88a0-76a27b8eab57` p. 194 — rule 2: J.,U! yute con ventaja, en aquellos aspecto& que pera éste presentaba..n Lnconvení.ent.es y C011 e s1)0 cí.al, destino péi'" ra la e.l aborací.ón de arpillera; e stud í o éste que aún se encuent-r
15. `b7635184-220d-4887-88a0-76a27b8eab57` p. 187 — rule 2: culti1,ro e.e '", "; 1 e ,-¡ lJ " ~'j -,) ,"""--.' .. u \Ah) O., c.__ ., J.. -.11. de dicha especie 2D la zona. . - • G "-" bl c~o~ar·-J.~olJo __ - aJ~ las_ J.: - '" ~)lant~~ ¿, I....~~::l -- ~ll~.
16. `b7635184-220d-4887-88a0-76a27b8eab57` p. 440 — rule 2: gala y Assam estticlasificada como A A' r) lo que sig nifica: muy hÚDí€da, troptc-.l y lluvia adecuada en to das las estaciones. La zona del nordeste de la Argen tina (Misio~es y Co:rrierites), cOl1re
17. `b7635184-220d-4887-88a0-76a27b8eab57` p. 237 — rule 2: En los'g~stos'g~~eralesse debe tener en cuenta: . 1 .... G-astos' de local' y cont.r'LbucLonea .. Incluye 'el' al.quf.Ler del Local, JO' el ' int3ró~ del ca~ pital fijo invertido, en &1 si es propf
18. `b7635184-220d-4887-88a0-76a27b8eab57` p. 339 — rule 2: la interior de arpillera cOD; atra cenñecc í.enada con hilado de éll.god~n,tamt>1~nc.nital.s resultados. La producción media del' quinquenio 1938/1942 de extracte de quebrach. y urunday J que llegó
19. `b7635184-220d-4887-88a0-76a27b8eab57` p. 1477 — rule 2: B.i '1942 .. Decreto 118.'15" d.lalt......2.ni.poItÍendo la .x prciplac14a de .bolo. U.pda en .1.' yapor"lñ.t bIank".B.O.16-5-42.- 1942 - .Arp11188_.. N.jando en. O. os &Ifn.. .1 re4lU'&O, pot' Ta
20. `b7635184-220d-4887-88a0-76a27b8eab57` p. 438 — rule 2: Ac@ntinu,íilci4rf se com.para 1... precipitaci'n plu vi~1 en mil!ll1etresdllrante 'el'per!G'd.' vegetativo, es decir. desde agost'. eh.re inclusive• Tetal . Prftcipi~"'cl'n pluvial. enrnil:fmetros
21. `b7635184-220d-4887-88a0-76a27b8eab57` p. 196 — rule 2: Les distintos pafse s de JLrnérica que PC)!- su sí t.ua ción geográfica tienen plffiltas indígenas productoras (1e C.Oli·~orius en 1.• 000 lu2. )r 1], ele C.Cél~)Glll.8_rif 3 811_ i~t12]. su..pe r-f'
22. `b7635184-220d-4887-88a0-76a27b8eab57` p. 56 — rule 2: -, Gerieralidade$ .El yute ha crecido en la India desde tiemPos in memoriales y los tejidosh~chos con el mismphan sido utilizados porel.pueblo.nativo.des~elos·p~imeros tiempos. Hoy como siempr~, p
23. `b7635184-220d-4887-88a0-76a27b8eab57` p. 374 — rule 2: econ'mic~s.~plicad~s~a nuestro.pa!s, Ílluchos vapores - prccedentesdeCalcuta) llegaron a nuestros puertos - con su bodega en l~stre,para tom~r carg~ argentina de cereales. Actualmente el problema
24. `b7635184-220d-4887-88a0-76a27b8eab57` p. 415 — rule 2: , -. En,'esta 1.c~lid{td, salveral"as excepciones,los c,! Lenss se encuentran decepcionados cen e ste cultive. 'Tt"mb~:En un establecimiento se sembraron en la camp.... fi~ 194.3/1944, 2'.50 hect{re
25. `b7635184-220d-4887-88a0-76a27b8eab57` p. 1065 — rule 2: Cuadro HO ¡~ 71-Arpillera (He aaí.an] :10,1/2 onzas en 40 -1- pulgadas:Precio mensual por yarda en Dundee.-Enero 1919/Abri1 1945 (Cont.) .Jío Enero febrero l"'4ar..,[O Abril Mayo Junio - en centa
26. `b7635184-220d-4887-88a0-76a27b8eab57` p. 1381 — rule 2: :LOa, ~."it1c.a el pUQto nx .~ p..... tU:· .··~,."~-t1noa rq_ de 1&1- ~t~tda"f ••lO, f¡Uttre&$pocttaal -.f:~ y 1& "~la&, ..1d¡)ulao~.80bre.fl~p cetn~ .. el rAJa" :II.- il,.~~.:_......en loaputirt
27. `b7635184-220d-4887-88a0-76a27b8eab57` p. 535 — rule 2: .. ~i !t&pleteriodeSaludPl1blica de la NaGi~h, s~nperjuiciod.e 1.-. 1\1¡ii.c::~on$s,ae&rda.da$ por ley nO 13. 266 de.,,f.en,~acon:t,r .. el· ;paludj.$J.to. deberáadup"'" tarl.slIedida.', .'rt"cf¡)s
28. `b7635184-220d-4887-88a0-76a27b8eab57` p. 198 — rule 2: 'Tei""!.·e-- r;'1,nl .~ Col ombí s ~·~c·;'a· dar ·'~r·t-:s~·ll t \¡ 1.1. I:..! ~L '-' .......C-l. B oJ -- .¡" _ c.... , -'--J "..4. ,.u o. __ _. , La euper-f'Lcí.e e oc e e he.da en. c ada l)fl:cceJ.e
29. `b7635184-220d-4887-88a0-76a27b8eab57` p. 368 — rule 2: .' presente al mismo tiem.pc> que .l.s Intsrnes e staban afecta d.es en ese enton<:es'altransporte marítimo en zonas me nes peligrosas y dedic¡¡,desal transporte de otros mate riales igualm~nteimport
30. `b7635184-220d-4887-88a0-76a27b8eab57` p. 254 — rule 2: .. .. . ... .. •.... "t ,,", v • '-.. ... ..~ . , ' <t .: o.oP"'" • '; o''' . -~nfardelado Be efectl1a' para··hacerniás 'e~onó~?-co el: tra~spor- "O' '. te'•.Esta oper-acá.ón 'se realiza' .con pr

### The three singleton flags (every flagged page of their attachments)

- `6f80900a-5f63-41d1-b839-3fc855c24516` p. 24 — rule 2: Error in Spreadsheet-embedded data: Data Type Error Table1: sqft_lot15,sqft_lot,waterfront,sqft_basement,view,yr_built,zipcode,price,yr_renovated,Unnamed: 0 5650,5650,0,0,0,1955,98178,221900.0,0,0
- `77cbd5ad-759b-4f69-b41c-51bef444aca6` p. 20 — rule 2: 202 Isabel Quintas Pereira ISSN impreso: 0188-7742 Política y Cultura, enero-junio 2020, núm. 53, pp. 183-204 |==========================================| 100% 13 MB > head (datos) # muestra encabe
- `820a66bc-e7b5-4a9b-a3a0-822c508be7ed` p. 246 — rule 2: HAN 12-ch05-187-242-9780123814791 2011/6/1 3:19 Page 209 #23 5.2 Data Cube Computation Methods 209 Algorithm: Star-Cubing. Compute iceberg cubes by Star-Cubing. Input: R: a relational table min s

These are clean pages carrying code or column names with dots (R's `read.csv`-style names);
they are the detector's only visible false positives in the whole library.

## Reading of the results

- The stored library's glued-text damage (rule 1 over stored rows: 1,479 pages in 82
  attachments) disappears under PDFium: rule 1 flags **zero** PDFium pages. This matches the
  plan's claim that PDFium resolves the spacing problem.
- What remains after PDFium is genuinely old-OCR noise (rule 2): one damaged scanned book
  (418 pages, all sampled pages visibly garbled) plus 3 clean data/code pages.
- Reprocess candidate sizing: the detector alone marks **1,610 stored page rows in 82
  attachments**; B3's candidate list will add the `sparse`/`empty`/`unreadable` rows the
  quality verdicts already cover (160 `empty`, 161 `sparse`, 1 `unreadable` in this library).

## Detector v2 (detector-old-ocr-noise, 2026-10-10)

After the real reprocess, Abulafia 1950 kept most of its 911 native pages as
old-OCR garbage that v1 missed: v1's rule 2 judged a page only with 40+ tokens
of 4+ letters and counted only `.`, `~`, `·` between letters, while this
garbage is punctuation soup with few whole words. v2 (see
`BIBLIOGRAPHY_DETECTOR_VERSION` in `ocr/pdf.rs`):

- Rule 2 widened: a digit, U+FFFD or one of `. ~ · " ; , : ! | ^ ` \ { } < >`
  between two letters; identifier-like tokens (URLs, DOIs, arXiv ids, paths,
  2+ digits) excluded; flags at 10 % over 10+ judged tokens.
- Rule 3 new, punctuation soup: 15 % of 20+ whitespace tokens carry no letter
  or digit and either a quote/tilde-like mark or 2+ distinct punctuation chars.
- A case-soup clause ("PrelUlliIl") was measured and dropped: it added 40
  Abulafia pages but flagged camelCase identifiers of born-digital papers in
  28 more works.

Same database copy method as above (backup of prueba-sync, 587 PDF attachments,
19,606 stored rows, 19,074 stored-`rich` pages read with PDFium):

| | v1 | v2 |
|---|---|---|
| PDFium pages flagged | (see above) | 1,126 (5.90 %) in 8 attachments |
| Stored rows flagged | 1,610 in 82 attachments | 599 in 21 attachments |

PDFium flags per attachment, read by hand:

| Attachment | Pages | Reading |
|---|---|---|
| Abulafia 1950, El problema del yute | 1,036 | old OCR garbage (true positive) |
| Paz 2016 | 59 | shifted glyph font (true positive) |
| Kabat et al. 2014 | 25 | substituted glyphs in the body (true positive) |
| Girbal-Blacha 2017 | 1 | ciphered font layer (true positive) |
| PaddleOCR 3.0 report, TableLLM, Kjell et al., Pereira 2020 | 5 | code, CSV, R console output (false positives) |

False positives: 5 born-digital pages of 19,074 (0.026 %), all code or data
that GLM-OCR also reads well. The regular extraction sends flagged pages to
GLM-OCR automatically (owner rule for scanned books with a bad text layer), so
this rate is what that rule costs on a born-digital library.
