import {StrictMode} from "react"
import {createRoot, type Root} from "react-dom/client"

import {App} from "@/app"
import "@/index.css"

const container: HTMLElement | null = document.getElementById("root")
if (!container) {
  throw new Error("The root element is missing")
}

const root: Root = createRoot(container)
root.render(
  <StrictMode>
    <App />
  </StrictMode>,
)
