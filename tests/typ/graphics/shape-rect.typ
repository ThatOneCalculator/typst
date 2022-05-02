// Test the `rect` function.

---
// Default rectangle.
#rect()

---
#set page(width: 150pt)

// Fit to text.
#rect(fill: conifer, inset: 3pt)[Textbox]

// Empty with fixed width and height.
#block(rect(
  height: 15pt,
  fill: rgb("46b3c2"),
  stroke: 2pt + rgb("234994"),
))

// Fixed width, text height.
#rect(width: 2cm, fill: rgb("9650d6"), inset: 5pt)[Fixed and padded]

// Page width, fixed height.
#rect(height: 1cm, width: 100%, fill: rgb("734ced"))[Topleft]

// These are inline with text.
\{#rect(width: 0.5in, height: 7pt, fill: rgb("d6cd67"))
  #rect(width: 0.5in, height: 7pt, fill: rgb("edd466"))
  #rect(width: 0.5in, height: 7pt, fill: rgb("e3be62"))\}

// Rounded corners.
#rect(width: 2cm, radius: 60%)
#rect(width: 1cm, radius: (x: 5pt, y: 10pt))
#rect(width: 1.25cm, radius: (left: 2pt, top: 5pt, right: 8pt, bottom: 11pt))

// Different strokes.
[
  #set rect(stroke: (right: red,))
  #rect(width: 100%, fill: lime, stroke: (x: 5pt, y: 1pt))
]

---
// Outset padding.
#let inline-code(body) = [
  #set text("IBM Plex Mono", 8pt)
  #h(.7em, weak: true)
  #rect(radius: 3pt, outset: (y: 3pt, x: 2.5pt), fill: rgb(239, 241, 243), body)
  #h(.7em, weak: true)
]

Use the #inline-code[\*const ptr] pointer.

---
// Error: 15-38 unexpected key "cake"
#rect(radius: (left: 10pt, cake: 5pt))

