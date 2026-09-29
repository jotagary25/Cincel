# Empaquetado de Cincel

Scripts y metadatos para producir los dos paquetes de la versión 1.0
(`docs/specs/08-etapa6-cierre-1-0.md` §6):

- `install.sh`: instalador POSIX `sh` para el tarball (se copia dentro de
  él, ver `build.sh`).
- `docker/Dockerfile.build` + `build.sh`: compilan `cincel` en release
  dentro de `ubuntu:22.04` (glibc 2.35) y arman el tarball y el `.deb` en
  `dist/`.
- `verify.sh`: instala y corre ambos paquetes en contenedores limpios
  `ubuntu:22.04` y `ubuntu:24.04`, y comprueba `ldd`, tamaño y símbolos de
  glibc.
- `tests/install_test.sh`: prueba `install.sh` a fondo (lo corre
  `verify.sh`).
- `icons/`: el SVG del icono y los PNG generados con `icons/render.sh`.
- `linux/dev.cincel.Cincel.desktop`: la entrada de menú.
- `about.toml` + `about.hbs`: config y plantilla de `cargo about generate`
  para `THIRD-PARTY-LICENSES.html` (se genera en el release, no se
  versiona).

## Maintainer del `.deb`

`crates/cincel/Cargo.toml` trae un `maintainer` genérico
(`Cincel contributors <cincel@users.noreply.github.com>`): nunca un correo
real. Si querés que el `.deb` lleve tu propio correo privado de GitHub
(`<tu-usuario>@users.noreply.github.com`), `build.sh` lo toma, en este
orden, de:

1. la variable de entorno `CINCEL_MAINTAINER`, o
2. un archivo `packaging/maintainer.txt` (una sola línea, **ignorado por
   git**: creá el archivo vos, nunca lo commitees).

Sin ninguno de los dos, se usa el valor genérico de arriba. `build.sh`
restaura `crates/cincel/Cargo.toml` a su contenido original al terminar (o
si se interrumpe), así que el valor real nunca queda escrito en el árbol de
trabajo.

## Uso

```sh
packaging/build.sh    # compila y arma tarball + .deb en dist/
packaging/verify.sh    # instala y corre ambos paquetes en 22.04 y 24.04
```

Ambos necesitan Docker (el usuario en el grupo `docker`, sin `sudo`). Al
terminar la verificación, `target/ubuntu-22.04` (el `CARGO_TARGET_DIR` del
contenedor) se borra para no llenar el disco.
