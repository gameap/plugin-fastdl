import type { PluginDefinition } from '@gameap/plugin-sdk';
import AdminPage from './pages/AdminPage.vue';
import FastDLTab from './tabs/FastDLTab.vue';
import { translations } from './translations';

export const fastdlPlugin: PluginDefinition = {
  id: 'i3z7ix336msd4',
  name: 'FastDL',
  version: __PLUGIN_VERSION__,
  apiVersion: '1.0',
  author: 'GameAP',
  description: 'Fast downloads for GoldSource and Source game servers',
  translations,
  routes: [{
    path: '/',
    name: 'index',
    component: AdminPage,
    meta: { title: 'FastDL', requiresAuth: true, requiresAdmin: true },
  }],
  menuItems: [{
    section: 'admin',
    icon: 'download',
    text: '@:fastdl',
    route: { name: 'index' },
    order: 55,
    adminOnly: true,
  }],
  slots: {
    'server-tabs': [{
      component: FastDLTab,
      order: 65,
      label: '@:fastdl',
      icon: 'download',
      name: 'fastdl',
      checkPermission: { type: 'hasServerPermissions', permissions: ['plugin:i3z7ix336msd4:fastdl-view'] },
      checkGame: { engines: ['GoldSource', 'Source'] },
    }],
  },
};
