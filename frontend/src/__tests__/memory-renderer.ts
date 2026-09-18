import { createRenderer, defineComponent, h, nextTick, type VNode } from 'vue';

export interface Element {
  tag: string;
  text: string;
  props: Record<string, unknown>;
  children: Element[];
  parent: Element | null;
}

const element = (tag: string, text = ''): Element => ({ tag, text, props: {}, children: [], parent: null });
const renderer = createRenderer<Element, Element>({
  createElement: (tag) => element(tag),
  createText: (text) => element('#text', text),
  createComment: (text) => element('#comment', text),
  setText: (node, text) => { node.text = text; },
  setElementText: (node, text) => { node.text = text; node.children = []; },
  parentNode: (node) => node.parent,
  nextSibling: (node) => node.parent?.children[(node.parent?.children.indexOf(node) ?? -1) + 1] ?? null,
  patchProp: (node, key, _, value) => { node.props[key] = value; },
  insert(node, parent, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
    const position = anchor ? parent.children.indexOf(anchor) : parent.children.length;
    parent.children.splice(position, 0, node);
    node.parent = parent;
  },
  remove(node) {
    node.parent?.children.splice(node.parent.children.indexOf(node), 1);
    node.parent = null;
  },
});

export const descendants = (node: Element): Element[] => [node, ...node.children.flatMap(descendants)];
export const content = (node: Element): string => node.text + node.children.map(content).join('');
export const find = (root: Element, tag: string, label?: string): Element => {
  const match = descendants(root).find((node) => node.tag === tag && (label === undefined || content(node).includes(label)));
  if (!match) throw new Error(`Missing ${tag} ${label ?? ''}`);
  return match;
};
export const invoke = (node: Element, event: string, value?: unknown) => (node.props[event] as (value?: unknown) => unknown)(value);
export const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
};
export async function settle() {
  for (let attempt = 0; attempt < 5; attempt++) { await Promise.resolve(); await nextTick(); }
}
export async function mountComponent(render: () => VNode) {
  const app = renderer.createApp({ setup: () => render });
  for (const name of ['GButton', 'GIcon', 'GStatusBadge', 'GModal', 'GBreadcrumbs', 'GEmpty']) {
    app.component(name, defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h(name, attrs, slots.default?.()) }));
  }
  const root = element('root');
  app.mount(root);
  await settle();
  return { root, app };
}
