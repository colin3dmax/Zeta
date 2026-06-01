import { mount } from "svelte";
import PlaygroundSection from "./PlaygroundSection.svelte";

function mountPlaygrounds() {
  document.querySelectorAll("zeta-playground").forEach((target) => {
    if (target.__zetaPlaygroundMounted) return;
    target.__zetaPlaygroundMounted = true;
    const component = mount(PlaygroundSection, {
      target,
      props: {
        initialExample: target.getAttribute("example") || "overview",
        embedded: true,
      },
    });
    target.__zetaPlaygroundLoadExample = (name) => component.loadExternalExample?.(name);
  });
}

document.addEventListener("click", (event) => {
  const trigger = event.target.closest?.("[data-zeta-example]");
  if (!trigger) return;
  const playground = document.querySelector(trigger.getAttribute("data-zeta-target") || "zeta-playground");
  if (!playground?.__zetaPlaygroundLoadExample) return;
  event.preventDefault();
  playground.__zetaPlaygroundLoadExample(trigger.getAttribute("data-zeta-example") || "overview");
  playground.scrollIntoView({ block: "start", behavior: "smooth" });
});

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", mountPlaygrounds);
} else {
  mountPlaygrounds();
}
