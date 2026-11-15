// KAIT2EN website - shared JavaScript

// Mobile Navigation Modal
function toggleMenu() {
    const modal = document.getElementById('nav-modal');
    if (modal) {
        modal.classList.toggle('open');
        document.body.style.overflow = modal.classList.contains('open') ? 'hidden' : '';
    }
}

function closeMenu() {
    const modal = document.getElementById('nav-modal');
    if (modal) {
        modal.classList.remove('open');
        document.body.style.overflow = '';
    }
}

// Lightbox
function openLightbox(img) {
    const lightbox = document.getElementById('lightbox');
    if (!lightbox) return;
    const lightboxImg = document.getElementById('lightbox-img');
    lightboxImg.src = img.src;
    lightboxImg.alt = img.alt;
    lightbox.classList.add('open');
    document.body.style.overflow = 'hidden';
}

function closeLightbox() {
    const lightbox = document.getElementById('lightbox');
    if (!lightbox) return;
    lightbox.classList.remove('open');
    document.body.style.overflow = '';
}

// Images written in Markdown open in the lightbox, without any per-image markup.
document.addEventListener('click', (e) => {
    const img = e.target.closest('.page-content img, .concept-story img');
    if (img && !img.closest('a')) openLightbox(img);
});

document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
        closeLightbox();
        closeMenu();
    }
});

// Copy Code Button
function copyCode() {
    const code = document.getElementById('install-code');
    if (!code) return;
    navigator.clipboard.writeText(code.textContent).then(() => {
        const btn = document.querySelector('.copy-btn');
        btn.innerHTML = `
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16">
                <polyline points="20 6 9 17 4 12"/>
            </svg>
            Copied!
        `;
        setTimeout(() => {
            btn.innerHTML = `
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16">
                    <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
                    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
                </svg>
                Copy
            `;
        }, 2000);
    });
}

// Collapsible sub-entries in the documentation navigation
function toggleNavChildren(button, open) {
    const list = button.nextElementSibling;
    const show = open === undefined ? list.hidden : open;
    list.hidden = !show;
    button.setAttribute('aria-expanded', String(show));
}

// Scroll Spy for Sidebar Navigation
document.addEventListener('DOMContentLoaded', function() {
    const navLinks = document.querySelectorAll('.page-nav a');
    if (!navLinks.length) return;

    const sections = document.querySelectorAll('section[id]');
    if (!sections.length) return;

    function setActiveLink(id) {
        navLinks.forEach(link => {
            link.classList.remove('active');
            if (link.getAttribute('href') === '#' + id) {
                link.classList.add('active');
                const parent = link.closest('.nav-parent');
                if (parent) toggleNavChildren(parent.querySelector('.nav-toggle'), true);
            }
        });
    }

    function updateActiveLink() {
        let current = '';
        const scrollPos = window.scrollY + 100;

        sections.forEach(section => {
            const top = section.offsetTop;
            const height = section.offsetHeight;
            if (scrollPos >= top && scrollPos < top + height) {
                current = section.getAttribute('id');
            }
        });

        setActiveLink(current);
    }

    // Update on scroll
    window.addEventListener('scroll', updateActiveLink);

    // Update on click
    navLinks.forEach(link => {
        link.addEventListener('click', function() {
            const id = this.getAttribute('href').substring(1);
            setActiveLink(id);
        });
    });

    updateActiveLink();
});

// Copy Button on documentation and blog code blocks
function copyBlock(btn) {
    const wrap = btn.closest('.code-wrap');
    if (!wrap) return;
    const code = wrap.querySelector('code');
    if (!code) return;
    navigator.clipboard.writeText(code.textContent).then(() => {
        const original = btn.innerHTML;
        btn.innerHTML = `
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="14" height="14">
                <polyline points="20 6 9 17 4 12"/>
            </svg>
            Copied!
        `;
        setTimeout(() => { btn.innerHTML = original; }, 2000);
    });
}

function copyDonation(btn) {
    const value = btn.dataset.copy;
    if (!value) return;
    navigator.clipboard.writeText(value).then(() => {
        const original = btn.innerHTML;
        btn.textContent = 'Copied!';
        setTimeout(() => { btn.innerHTML = original; }, 2000);
    });
}

// Project filter on the patches page. Without JavaScript every patch stays
// visible and only the chips are inert.
document.addEventListener('DOMContentLoaded', function () {
    const chips = document.querySelectorAll('.pt-chip');
    if (!chips.length) return;
    const items = document.querySelectorAll('.pt-item');
    const groups = document.querySelectorAll('.pt-group');
    const empty = document.querySelector('.pt-empty');

    chips.forEach(function (chip) {
        chip.addEventListener('click', function () {
            const project = chip.dataset.project;
            chips.forEach(function (other) {
                const active = other === chip;
                other.classList.toggle('is-active', active);
                other.setAttribute('aria-pressed', active ? 'true' : 'false');
            });
            items.forEach(function (item) {
                item.hidden = project !== 'all' && item.dataset.project !== project;
            });
            let visible = 0;
            groups.forEach(function (group) {
                group.hidden = !group.querySelector('.pt-item:not([hidden])');
                if (!group.hidden) visible++;
            });
            if (empty) empty.hidden = visible !== 0;
        });
    });
});

// A link to a collapsed entry (a patch, a FAQ question) opens it.
function openTarget() {
    const target = location.hash && document.getElementById(decodeURIComponent(location.hash.slice(1)));
    if (target && target.tagName === 'DETAILS') target.open = true;
}
document.addEventListener('DOMContentLoaded', openTarget);
window.addEventListener('hashchange', openTarget);

// The MacBook in the landing page hero closes while the hero scrolls past.
document.addEventListener('DOMContentLoaded', function () {
    const hero = document.querySelector('.home-hero');
    const stage = hero && hero.querySelector('.home-stage');
    if (!stage || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;

    // Centre the stage in the viewport below the navbar.
    const bar = document.querySelector('.navbar');
    function place() {
        const navbar = bar ? bar.offsetHeight : 0;
        const top = Math.max(navbar + 24, (window.innerHeight + navbar - stage.offsetHeight) / 2);
        hero.style.setProperty('--stage-top', Math.round(top) + 'px');
    }

    let queued = false;
    function update() {
        queued = false;
        const top = parseFloat(hero.style.getPropertyValue('--stage-top')) || 0;
        const range = hero.offsetHeight - top - stage.offsetHeight;
        const done = range > 0 ? -hero.getBoundingClientRect().top / range : 0;
        hero.style.setProperty('--lid', Math.min(1, Math.max(0, done)).toFixed(4));
    }

    window.addEventListener('scroll', function () {
        if (!queued) {
            queued = true;
            requestAnimationFrame(update);
        }
    }, { passive: true });
    window.addEventListener('resize', function () {
        place();
        update();
    });
    place();
    update();
});
