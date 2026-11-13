# SharkPaint

Aplicación nativa de dibujo y pintura digital para Windows, escrita en Rust.
No usa navegador embebido: se compila a un único `SharkPaint.exe`.

## Estado actual (fase 1)

- Lienzo con zoom, desplazamiento, rotación y volteo horizontal.
- Pincel con sensibilidad a la presión del lápiz (tamaño y flujo).
- Cuatro pinceles iniciales: Lápiz, Tinta, Marcador y Aerógrafo, todos ajustables
  (tamaño, opacidad, flujo, dureza, espaciado y estabilizador).
- Borrador, cuentagotas, deshacer y rehacer (200 pasos).
- Exportación a PNG.

## Pendiente

- Capas con modos de fusión.
- Panel de referencias: imágenes de consulta en un panel propio, fuera del lienzo y
  de las capas, con zoom y desplazamiento independientes y cuentagotas sobre la
  referencia. No se incluyen al exportar.
- Guardado de proyecto e instalador.

## Atajos

| Atajo | Acción |
|---|---|
| `B` / `E` | Pincel / Borrador |
| `[` / `]` | Reducir / aumentar el tamaño del pincel |
| `Alt` + clic | Tomar color del lienzo |
| `Ctrl+Z` / `Ctrl+Mayús+Z` | Deshacer / Rehacer |
| `Espacio` + arrastrar, botón central | Mover el lienzo |
| Rueda del ratón, `Ctrl` + `+` / `-` | Zoom del lienzo |
| `R` + arrastrar, `4` / `6` | Rotar el lienzo |
| `5` | Restablecer la rotación |
| `M` | Voltear horizontalmente |
| `Ctrl+0` / `Ctrl+1` | Ajustar a la ventana / 100 % |
| `Ctrl+S` | Exportar PNG |
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

El ejecutable queda en `target/release/SharkPaint.exe`. Cada push también genera el
ejecutable de Windows en GitHub Actions (artefacto `SharkPaint-windows-x64`).

```bash
cargo test
```

## Estructura

- `src/engine/`: motor de pintura (capas en mosaicos, pincel, historial), sin
  dependencias de interfaz.
- `src/app.rs`, `src/view.rs`: interfaz de escritorio y navegación del lienzo.
- `examples/muestra.rs`: genera una hoja de muestra de los pinceles sin abrir ventana.
