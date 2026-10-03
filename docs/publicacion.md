# Publicar Cincel en GitHub

Guía paso a paso para el autor (`docs/specs/08-etapa6-cierre-1-0.md` §7.3).
Nadie más hace estos pasos: crear el repositorio, decidir la historia de
git, empujar y publicar la primera versión son decisiones que le
corresponden a la persona dueña de la cuenta.

Antes de arrancar, corré (si no lo corrió ya el orquestador):

```sh
tools/privacy-check.sh
```

## 1. Leer el resultado del control de datos personales

`tools/privacy-check.sh` revisa el árbol de trabajo (debería salir **con
0**: si encuentra algo ahí, es un error real, corregilo antes de seguir) y
la historia completa de git, que informa pero nunca hace fallar el
script ni la reescribe (`tools/privacy-check.sh`, cabecera). En esta etapa
encontró lo esperado en la historia: el correo de autoría de los 5 commits
existentes no es un correo de ejemplo (es el que tenías configurado en
`git config user.email` al hacer esos commits). Nada de eso se corrigió
acá — es exactamente la decisión del paso 2.

Si el control encuentra algo en el **árbol de trabajo**, arreglalo (es un
archivo nuevo o modificado con un dato personal) y volvé a correr el
script antes de seguir.

## 2. Decidir la historia de git

Dos caminos:

**A) Publicar la historia tal cual** (5 commits, con tu nombre y tu correo
real de esos commits visibles para siempre en el repositorio público):

```sh
# no hace falta ningún comando: simplemente no reescribís nada
```

**B) Empezar una historia nueva de un solo commit** (recomendado si no
querés que tu correo real quede público, o si preferís no exponer el
detalle de cómo se construyó cada etapa):

```sh
git checkout --orphan historia-nueva
git add -A
git commit -m "feat: Cincel 0.2.1"
git branch -D main 2>/dev/null || true
git branch -m main
```

Con cualquiera de las dos, el paso 3 (correo del commit) sigue aplicando
a partir de ahora.

## 3. Configurar el correo privado de GitHub para los commits que se publiquen

En GitHub, Configuración → Emails → activá "Keep my email address
private"; te da una dirección `<tu-usuario>@users.noreply.github.com`.
Usala para lo que publiques de acá en adelante:

```sh
git config user.email "<tu-usuario>@users.noreply.github.com"
```

(Esto no toca los commits que ya existen si elegiste la opción A del
paso 2; solo los que hagas de ahora en más.)

## 4. Crear el repositorio en GitHub

Nombre sugerido: `cincel` (D22). Público. **Sin** README, licencia ni
`.gitignore` iniciales — ya están en este repo.

Por la web (github.com → New repository), o con `gh`:

```sh
gh repo create cincel --public --source . --remote origin
```

## 5. Completar `jotagary25` en todo el repo

Varios archivos tienen el marcador `jotagary25` en la URL
`https://github.com/jotagary25/cincel` (D22): `Cargo.toml` (campos
`repository` y `homepage`), `README.md`, `SECURITY.md` y
`.github/ISSUE_TEMPLATE/config.yml`. Para verlos todos:

```sh
grep -rn "jotagary25" --include="*.md" --include="*.toml" --include="*.yml" .
```

Reemplazalos todos con tu usuario real de GitHub:

```sh
grep -rl "jotagary25" --include="*.md" --include="*.toml" --include="*.yml" . \
    | xargs sed -i "s/jotagary25/TU-USUARIO-DE-GITHUB/g"
```

(o pedíselo a Cincel/Claude, dándole tu usuario en el chat).

Después de reemplazar, corré `tools/check-links.sh` de nuevo si tocaste
algún enlace a mano.

## 6. Primer push

```sh
git push -u origin main
```

Si preferís trabajar con una rama `develop` además de `main` (por ejemplo
para no tocar `main` directo en cambios grandes), creala ahora:

```sh
git branch develop
git push -u origin develop
```

`ci.yml` corre en push a `main` y en cada pull request; no hace falta que
`develop` tenga su propio workflow.

## 7. Ajustes del repositorio (en GitHub, Configuración del repo)

- **Actions**: habilitado, con permisos de lectura por defecto para
  `GITHUB_TOKEN` (Settings → Actions → General → Workflow permissions →
  "Read repository contents permission").
- **Protección de la rama `main`**: Settings → Branches → Add rule:
  - "Require status checks to pass before merging" → marcá los jobs de
    `ci.yml` (`fmt`, `clippy`, `test`, `deny`, `shellcheck`).
  - "Restrict deletions" y sin *force push* (dejá desmarcado "Allow force
    pushes").
  - Podés dejarte a vos mismo la opción de seguir empujando directo a
    `main` (no marcar "Require a pull request before merging" si preferís
    seguir así); los checks igual corren en cada push.
- **Reporte privado de vulnerabilidades**: Settings → Security → activá
  "Private vulnerability reporting" (es el canal que usan `SECURITY.md` y
  `CODE_OF_CONDUCT.md`).
- **Descripción y temas**: en la página principal del repo, ⚙️ junto a
  "About" → descripción corta (por ejemplo, la misma línea que
  `description` en `Cargo.toml`) y temas: `editor`, `rust`, `gpui`, `acp`,
  `ai-agents`.
- **Secretos**: ninguno hace falta. `release.yml` usa el
  `GITHUB_TOKEN` que GitHub genera solo para cada corrida (permiso
  `contents: write` ya está declarado en el workflow, en el job que
  publica el borrador).

## 8. Esperar el CI en verde

Cada push a `main` y cada pull request dispara `.github/workflows/ci.yml`
(pestaña **Actions** del repo). Si algo falla:

1. Abrí el job que falló y copiá el mensaje de error (sin datos
   personales: rutas de tu casa, nombre de tu equipo).
2. Pegáselo a Cincel/Claude con el nombre del job y el paso.

## 9. Publicar una versión (release)

Antes de etiquetar, en el mismo commit de la versión: `version` en
`[workspace.package]` de `Cargo.toml`, la entrada nueva de `CHANGELOG.md`,
un `<release>` nuevo en `packaging/linux/dev.cincel.Cincel.metainfo.xml`
(fecha y enlace a la release) y los nombres de archivo de ejemplo del
`README.md`.

Con el CI en verde en `main`:

```sh
git tag -a v0.2.1 -m "Cincel 0.2.1"
git push origin v0.2.1
```

Esto dispara `.github/workflows/release.yml`: compila el binario de
versión, arma el tarball y el `.deb`, los verifica en contenedores
limpios (`ubuntu:22.04` y `ubuntu:24.04`) y deja un **borrador** de
release con los cuatro archivos (`cincel-0.2.1-x86_64-linux.tar.gz` +
`.sha256`, `cincel_0.2.1-1_amd64.deb` + `.sha256`).

Antes de publicarlo:
- Descargá el `.deb` del borrador y probalo en tu máquina
  (`sudo apt install ./cincel_0.2.1-1_amd64.deb`, `cincel --version`,
  abrilo).
- Revisá las notas (se arman solas desde `CHANGELOG.md`).
- En la página del borrador, botón **"Publish release"**.

Si el tag no coincide con la versión de `[workspace.package]` en
`Cargo.toml`, el job `build` falla antes de compilar nada (mensaje "el tag
... no coincide con la versión ... de Cargo.toml"): corregí el tag o la
versión y volvé a etiquetar.

## 10. Opcional: renombrar la carpeta local

Si tu carpeta local todavía se llama distinto (por ejemplo
`asteroid-editor`, de antes de que el proyecto se llamara Cincel,
`pendientes-etapa-5.md §A.7`):

```sh
cd ..
mv asteroid-editor cincel
cd cincel
```

Si tenías atajos del sistema (lanzador, accesos directos, un alias de
shell) apuntando a la carpeta vieja, actualizalos a la ruta nueva.

## 11. Qué NO hacer

- No subas `~/.config/cincel`, `~/.local/share/cincel`,
  `~/.local/state/cincel` ni `~/.cache/cincel`: son tu configuración y tus
  conexiones, no forman parte del repositorio (ya están fuera de él; nunca
  los agregues a mano).
- No pegues logs sin revisar en un issue o en un commit — pasalos primero
  por el mismo criterio que `tools/privacy-check.sh` (rutas de tu casa,
  tu usuario, tokens).

## Sin Docker

Si tu máquina no tiene Docker (no es el caso en la máquina de
referencia), `packaging/build.sh` y `packaging/verify.sh` corren igual con
`podman` con los mismos comandos (`alias docker=podman`, o
`ln -s "$(command -v podman)" ~/.local/bin/docker` si preferís no tocar el
script), o en una máquina virtual con Ubuntu 22.04 siguiendo
`packaging/docker/Dockerfile.build` a mano.
