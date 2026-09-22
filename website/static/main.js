// DeQ Website - Shared JavaScript

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

// Product Gallery Switcher
function switchGalleryImage(thumb) {
    const gallery = thumb.closest('.product-gallery');
    if (!gallery) return;
    const main = gallery.querySelector('.product-gallery-main');
    const caption = gallery.querySelector('.product-gallery-caption');
    main.src = thumb.src;
    main.alt = thumb.alt;
    if (caption && thumb.dataset.caption) {
        caption.textContent = thumb.dataset.caption;
    }
    gallery.querySelectorAll('.product-gallery-thumb').forEach(t => t.classList.remove('active'));
    thumb.classList.add('active');
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

// Trust Quotes Crossfade
document.addEventListener('DOMContentLoaded', function() {
    const quotes = document.querySelectorAll('.trust-quote');
    if (!quotes.length) return;

    let currentIndex = 0;
    const interval = 5000;

    function showNextQuote() {
        quotes[currentIndex].classList.remove('active');
        currentIndex = (currentIndex + 1) % quotes.length;
        quotes[currentIndex].classList.add('active');
    }

    setInterval(showNextQuote, interval);
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

// Feature Board Filter. Without JavaScript every row stays visible and only
// the chips are inert.
document.addEventListener('DOMContentLoaded', function () {
    function setup(root) {
        const chips = root.querySelectorAll('.board-chip');
        const rows = root.querySelectorAll('.board-row');
        const empty = root.querySelector('.board-empty');

        function matches(row, filter) {
            if (filter === 'all') return true;
            if (filter === 'help') return row.dataset.help === 'yes';
            const parts = filter.split(':');
            return row.dataset[parts[0]] === parts[1];
        }

        function apply(filter) {
            let visible = 0;
            rows.forEach(function (row) {
                const show = matches(row, filter);
                row.hidden = !show;
                if (show) visible++;
            });
            if (empty) empty.hidden = visible !== 0;
        }

        chips.forEach(function (chip) {
            chip.addEventListener('click', function () {
                chips.forEach(function (other) {
                    const active = other === chip;
                    other.classList.toggle('is-active', active);
                    other.setAttribute('aria-pressed', active ? 'true' : 'false');
                });
                apply(chip.dataset.filter);
            });
        });
    }

    document.querySelectorAll('.board').forEach(setup);
});

// Lightbox for images inside rendered Markdown
document.addEventListener('DOMContentLoaded', function () {
    document.querySelectorAll('.page-content img').forEach(function (img) {
        img.classList.add('page-image');
        img.addEventListener('click', function () { openLightbox(img); });
    });
});
