# Fuentes embebidas

Descargadas de las releases oficiales de cada proyecto (no de un espejo ni de
un paquete de distribución) y usadas sin recortar (sin *subsetting*): la OFL
1.1 trata el recorte como una modificación
(`docs/specs/08-etapa6-cierre-1-0.md` D7).

## Inter 4.1

- Release: <https://github.com/rsms/inter/releases/tag/v4.1>
- Paquete descargado: `Inter-4.1.zip`
- Archivos usados (dentro de `extras/ttf/` del zip, estáticos, no variables):

| Archivo | SHA-256 |
|---|---|
| `Inter-Regular.ttf` | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| `Inter-Italic.ttf` | `bbc051dd204b5019a1aa0bc0ae2aa8a05ab13e7a3f979fa357631dc7feb6833a` |
| `Inter-Medium.ttf` | `97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872` |
| `Inter-SemiBold.ttf` | `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3` |
| `Inter-Bold.ttf` | `288316099b1e0a47a4716d159098005eef7c0066921f34e3200393dbdb01947f` |
| `Inter-BoldItalic.ttf` | `948405a16cdc62701da5f4005ed068ca5f4d27061d98f7974ccfc37831d9581d` |

Licencia: SIL Open Font License 1.1 (`inter/OFL.txt`, copiada tal cual del
`LICENSE.txt` del zip).

## JetBrains Mono 2.304

- Release: <https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304>
- Paquete descargado: `JetBrainsMono-2.304.zip`
- Archivos usados (dentro de `fonts/ttf/` del zip, estáticos, no variables):

| Archivo | SHA-256 |
|---|---|
| `JetBrainsMono-Regular.ttf` | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |
| `JetBrainsMono-Italic.ttf` | `9d0a1f7a708e6af183f1193b7e81d40da294f5c67682c085d8401c60aac8ded4` |
| `JetBrainsMono-Bold.ttf` | `5590990c82e097397517f275f430af4546e1c45cff408bde4255dad142479dcb` |
| `JetBrainsMono-BoldItalic.ttf` | `4039d5ce0ed225bf9c8b2c8c6436290ae2f356b7e90d70fa666227238324aa3b` |

Licencia: SIL Open Font License 1.1 (`jetbrains-mono/OFL.txt`, copiada tal
cual del `OFL.txt` del zip).

## Comprobación del hash

Ninguno de los dos proyectos publica un SHA-256 por archivo (ni siquiera del
zip completo: la API de releases de GitHub no trae `digest` para estos
assets). Los hashes de esta tabla son los que calculamos nosotros al
descargar, y sirven para notar si el archivo cambia más adelante
(`sha256sum crates/cincel-workspace/assets/fonts/**/*.ttf`); no hay un valor
publicado por Inter o JetBrains contra el cual contrastarlos. Desviación de
la letra de §4.2 ("se comprueba el hash contra el publicado cuando exista"):
no existe uno publicado para verificar.
