import "./styles.css";
import { mountAnalytics } from "./analytics";
import { mountTimer } from "./timer";

const analytics = new URLSearchParams(window.location.search).get("view") === "analytics";

(analytics ? mountAnalytics() : mountTimer()).catch((error) => {
  const app = document.querySelector<HTMLElement>("#app")!;
  app.innerHTML = `<main class="fatal-error"><strong>Focus Square 1.2</strong><p></p></main>`;
  app.querySelector("p")!.textContent = String(error);
});
