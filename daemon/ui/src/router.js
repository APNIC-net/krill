import Vue from 'vue'
import Router from 'vue-router'

import Home from './views/Home.vue'
import Login from './views/Login.vue'
import Publishers from './views/Publishers.vue'
import PublisherDetails from './views/PublisherDetails.vue'
import TrustAnchor from './views/TrustAnchor'
import APIService from "./services/APIService.js";

Vue.use(Router)

const router = new Router({
  routes: [
    {
      path: '/',
      name: 'home',
      component: Home
    },
    {
      path: '/publishers',
      name: 'publishers',
      component: Publishers
    },
    {
      path: '/ta',
      name: 'trustanchor',
      component: TrustAnchor
    },
    {
      path: '/login',
      name: 'login',
      component: Login
    },
    {
      path: '/publishers/:handle',
      name: 'publisherDetails',
      component: PublisherDetails
    },
  ]
});

router.beforeEach((to, from, next) => {
  const publicPages = ['/login'];
  const authRequired = !publicPages.includes(to.path);
  const loggedInOnFrontend = localStorage.getItem('user');

  if (authRequired) {
    APIService.isLoggedIn().then(loggedInOnBackend => {
      if (!loggedInOnFrontend || !loggedInOnBackend) {
        return next({
          path: '/login',
          query: {
            returnUrl: to.path
          }
        });
      }
      next();
    });
  } else {
    next();
  }
});

export default router;
