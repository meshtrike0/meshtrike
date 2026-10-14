// "Save as PDF": the browser's print dialog, where PDF is one of the destinations.
document.querySelectorAll('[data-print]').forEach((b) => b.addEventListener('click', () => window.print()));
