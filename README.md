# SharkPaint

Aplicación nativa de dibujo y pintura digital para Windows, escrita en Rust.
No usa navegador embebido: se compila a un único `SharkPaint.exe`.

## Estado actual

- Lienzo con zoom, desplazamiento, rotación y volteo horizontal.
- Pincel con sensibilidad a la presión del lápiz (tamaño y flujo).
- Diecinueve pinceles iniciales, de punta redonda (Lápiz, Tinta, Pluma, Marcador,
  Aerógrafo, Caligrafía, Pastel, Crayón), con forma (Carboncillo, Cabello, Pincel
  seco, Tiza, Salpicadura, Hojas, Estrellas) y de mezcla (Óleo, Acuarela,
  Difuminador, Dedo).
- Mezcla de color: el pincel puede recoger el color del lienzo y arrastrarlo, desde
  una pintura que se funde con la de debajo hasta un difuminador que no añade color.
- Textura de papel: cuatro papeles (fino, rugoso, de acuarela y lienzo) cuyo grano
  aparece según la presión del lápiz.
- Editor de pinceles con muestra del trazo: tamaño, opacidad, flujo, dureza,
  espaciado, estabilizador, forma de la punta, ángulo, redondez, dispersión,
  variación al azar, textura de papel y mezcla de color. La biblioteca muestra cada
  pincel con su trazo, agrupado por categorías y con buscador. Los pinceles propios se guardan en una biblioteca.
- Puntas importadas: cualquier imagen sirve como punta de pincel.
- Pinceles de Photoshop: importa archivos `.abr`, tanto los pinceles con imagen como
  los redondos (nombre, tamaño, dureza, espaciado, ángulo, redondez y si la presión
  controla el tamaño o el flujo). El resto de dinámicas de Photoshop no se trasladan.
- Capas: crear, duplicar, reordenar, fusionar y eliminar; opacidad, visibilidad,
  bloqueo de transparencia y modos de fusión (Normal, Multiplicar, Trama, Superponer,
  Oscurecer, Aclarar y Sumar). Cada capa muestra una miniatura de su contenido.
- Máscaras de recorte: una capa puede mostrarse solo donde la de debajo tiene dibujo.
- Copiar, cortar y pegar la selección o la capa (`Ctrl+C`, `Ctrl+X`, `Ctrl+V`).
- Herramientas: borrador, difuminar, relleno (con tolerancia y detección de líneas en
  todas las capas), cuentagotas, mover capa y transformar (escalar y rotar con una
  caja sobre el lienzo).
- Selección rectangular, elíptica o de forma libre: pintar, borrar, rellenar,
  transformar y voltear afectan solo a lo seleccionado.
- Voltear la capa o la selección, en horizontal y en vertical.
- Navegador (`F7`, pestaña junto al color): miniatura del lienzo completo que marca la zona visible;
  un toque lleva la vista a ese punto, y tiene controles de zoom y rotación.
- Panel de referencias (`F6`): imágenes de consulta en una ventana aparte, con zoom
  propio y cuentagotas. Se guardan con el proyecto y nunca se exportan.
- Insertar imágenes (PNG, JPEG, WebP, BMP, GIF) como capa nueva: desde un archivo,
  pegando con `Ctrl+V` o arrastrando el archivo a la ventana.
- Simetría: los trazos se repiten en espejo respecto al centro del lienzo.
- Paleta de colores guardados e historial de colores recientes.
- Autoguardado: tras un cierre inesperado, ofrece recuperar el dibujo.
- Atajos de herramientas configurables (Editar → Preferencias).
- Deshacer y rehacer (200 pasos, incluidos los cambios de capas).
- Gestos táctiles con dos dedos: mover, rotar y hacer zoom.
- Proyectos en formato propio (`.spaint`) que conservan la precisión completa del
  dibujo, y exportación a PNG.
- Aviso antes de descartar cambios sin guardar.

## Pendiente

- Grupos de capas.
- Sumar y restar selecciones, y deshacer los cambios de selección.
- Rediseño del icono (el pincel actual es provisional), junto con la actualización
  visual de SharkCut (versión 0.2.0).

## Atajos

| Atajo | Acción |
|---|---|
| `B` / `E` / `D` | Pincel / Borrador / Difuminar (configurables) |
| `G` / `I` / `V` | Relleno / Cuentagotas / Mover capa |
| `T` | Transformar la capa o la selección (`Intro` aplica, `Esc` cancela) |
| `S` | Selección (`Mayús`: cuadrado o círculo; un clic la quita) |
| `Ctrl+A` / `Ctrl+D` | Seleccionar todo / Deseleccionar |
| `Supr` | Borrar lo seleccionado |
| `[` / `]` | Reducir / aumentar el tamaño del pincel |
| `Alt` + clic | Tomar color del lienzo |
| `F5`, o clic en la herramienta activa | Mostrar u ocultar los ajustes del pincel |
| `Ctrl+Z` / `Ctrl+Mayús+Z` | Deshacer / Rehacer |
| `M` + arrastrar, `Espacio` + arrastrar, botón central | Mover el lienzo |
| Rueda del ratón, `Ctrl` + `+` / `-` | Zoom del lienzo |
| `R` + arrastrar, `4` / `6` | Rotar el lienzo |
| Dos dedos (pantalla táctil) | Mover, rotar y hacer zoom |
| `Ctrl+Mayús+N` / `Ctrl+J` | Nueva capa / Duplicar capa |
| `5` | Restablecer la rotación |
| `F` | Voltear horizontalmente |
| `F6` | Mostrar u ocultar las referencias |
| `F7` | Cambiar entre el color y el navegador |
| `F11` | Pantalla completa |
| `Ctrl+0` / `Ctrl+1` | Ajustar a la ventana / 100 % |
| `Ctrl+O` | Abrir proyecto |
| `Ctrl+S` / `Ctrl+Mayús+S` | Guardar / Guardar como |
| `Ctrl+E` | Exportar PNG |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copiar / Cortar / Pegar (también imágenes de otras aplicaciones) |
| `Ctrl+N` | Nuevo lienzo |

## Tableta gráfica

La presión se lee mediante Windows Ink. En el panel del controlador de la tableta
(Wacom, Huion, XP-Pen, etc.) la opción **Windows Ink** debe estar activada. La barra
de estado muestra la presión en cuanto la aplicación detecta el lápiz.

## Compilar

Requiere [Rust](https://rustup.rs).

```bash
cargo run --release
```

El ejecutable queda en `target/release/SharkPaint.exe`. Cada push también genera en
GitHub Actions el ejecutable (artefacto `SharkPaint-windows-x64`) y el instalador
(artefacto `SharkPaint-instalador`).

## Publicar una versión

1. Actualizar `version` en `Cargo.toml` y escribir `docs/novedades/<versión>.md`.
2. Crear y subir la etiqueta `v<versión>` sobre `main`.

La integración continua compila el instalador (`installer/SharkPaint.iss`, que asocia
los `.spaint`) y el `.zip` portable, y los publica en GitHub Releases. La página de
descarga (`web/`, publicada en GitHub Pages) muestra siempre la última versión.

```bash
cargo test
```

## Estructura

- `src/engine/`: motor de pintura (capas en mosaicos, pincel, historial), sin
  dependencias de interfaz.
- `src/app.rs`, `src/view.rs`: interfaz de escritorio y navegación del lienzo.
- `src/brushes.rs`: biblioteca de pinceles, su guardado e importación de puntas.
- `src/engine/tips.rs`, `src/engine/papers.rs`: puntas y papeles incluidos, generados
  por código.
- `src/settings.rs`: preferencias guardadas entre sesiones y archivo de recuperación.
- `src/icons.rs`: iconos de la interfaz (trazos vectoriales, mismo estilo que SharkCut).
- `src/engine/tools.rs`: relleno, difuminar, mover y transformar capa, insertar imágenes.
- `src/engine/selection.rs`: máscara de selección.
- `src/engine/project.rs`: formato de archivo de proyecto (documentado en el propio archivo).
- `examples/muestra.rs`, `examples/puntas.rs`, `examples/papeles.rs`, `examples/mezcla.rs`:
  generan hojas de muestra de los pinceles, las puntas, los papeles y la mezcla de
  color sin abrir ventana. `examples/abr.rs` lista y dibuja las puntas de un `.abr`.
- `tools/icono/generar_icono.py`: genera el icono (`assets/icons/`).

## Licencia y contribuciones

SharkPaint es software libre bajo la licencia GPL 3.0 (ver [LICENSE](LICENSE)). El
nombre y el icono tienen su propia política: ver [MARCA.md](MARCA.md).

No se aceptan *pull requests* externos; los reportes de errores y las ideas son
bienvenidos como *issues*. Ver [CONTRIBUTING.md](CONTRIBUTING.md).
