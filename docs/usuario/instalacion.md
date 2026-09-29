# Instalación

## Requisitos

- Ubuntu o Pop!_OS 22.04 o más nuevo (también sirve Ubuntu 24.04).
- Una tarjeta gráfica con Vulkan. Si no tenés una, Cincel no arranca y te lo
  dice; para forzarlo igual (con mal rendimiento) arrancalo con
  `CINCEL_ALLOW_SOFTWARE_GPU=1 cincel`. Más detalle en
  [Solución de problemas](problemas.md#sin-gpu-o-sin-vulkan).
- Nada más. Los agentes (Claude, Codex, Antigravity) y el Node privado que
  necesitan se descargan solos la primera vez que conectás cada uno; no
  vienen adentro del instalador.

Cincel no pide contraseña para instalarse ni para desinstalarse: todo vive
en tu carpeta personal.

## Opción 1: el archivo comprimido (`tar.gz`)

1. Descomprimí `cincel-1.0.0-x86_64-linux.tar.gz`.
2. Entrá a la carpeta que se creó y corré:

   ```sh
   ./install.sh
   ```

   Esto copia Cincel a `~/.local/bin/cincel`, agrega el icono y la entrada de
   menú (`~/.local/share/applications`), y deja la documentación en
   `~/.local/share/doc/cincel`. Al terminar, Cincel aparece en el lanzador de
   aplicaciones de tu escritorio, con su propio icono (un cincel).
3. Si `~/.local/bin` no está en tu `PATH`, el instalador te dice qué línea
   agregar a tu `~/.bashrc` o `~/.zshrc`.
4. Si falta alguna biblioteca del sistema, el instalador la nombra y te dice
   el paquete exacto para instalarla (por ejemplo
   `sudo apt install libxkbcommon-x11-0`); no falla por eso, solo avisa.

Para instalar en otro lugar: `./install.sh --prefix /otra/carpeta`.

### Desinstalar el comprimido

```sh
./install.sh --uninstall
```

Borra exactamente los archivos que instaló (el binario, el icono, la
entrada de menú y la documentación) y **nunca toca** tu configuración, tus
conexiones ni tus revisiones pendientes: esas siguen en `~/.config/cincel`,
`~/.local/share/cincel`, `~/.local/state/cincel` y `~/.cache/cincel`. El
instalador te lo recuerda al terminar; si además querés borrar eso, hacelo a
mano.

## Opción 2: el paquete `.deb`

Doble clic en `cincel_1.0.0-1_amd64.deb` (se abre con el instalador de
paquetes de tu escritorio) o, desde una terminal:

```sh
sudo apt install ./cincel_1.0.0-1_amd64.deb
```

Esta opción instala Cincel para todo el sistema, bajo `/usr`, y `apt`
resuelve solo las bibliotecas que le falten. Para desinstalarlo:

```sh
sudo apt remove cincel
```

Tu configuración y tus conexiones tampoco se tocan al desinstalar el `.deb`,
por la misma razón: viven en tu carpeta personal, no en `/usr`.

## Actualizar a una versión nueva

No hay un botón de "actualizar" dentro de Cincel. Para pasar a una versión
nueva: descargá el paquete nuevo y repetí la instalación (el tarball
sobrescribe lo que ya estaba; el `.deb` se actualiza con
`sudo apt install ./cincel_<versión nueva>_amd64.deb`). Tu configuración,
tus conexiones y tus revisiones pendientes no se pierden: no están en el
paquete, están en tu carpeta personal.

## ¿De dónde salió mi ejecutable?

Si instalaste las dos formas alguna vez (o ya tenías otra `cincel` en tu
`PATH`), puede haber más de un binario. `install.sh` te avisa si detecta
esa situación y te dice cuál se va a ejecutar quién primero en tu `PATH`.
Para estar seguro de cuál corrés: `which cincel` y `cincel --version`.
