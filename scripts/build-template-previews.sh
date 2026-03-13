#!/usr/bin/env bash
# Run this once in local dev to pre-compile template preview PDFs.
# In Docker, this happens automatically at image build time.
set -e
TEMPLATES_DIR="apps/api/templates"
for dir in "$TEMPLATES_DIR"/*/; do
    if [ -f "$dir/preview-source.tex" ]; then
        echo "Compiling: $dir (preview-source.tex)"
        (cd "$dir" && tectonic preview-source.tex --outfmt pdf && mv preview-source.pdf preview.pdf)
        echo "  -> preview.pdf generated from preview-source.tex"
    elif [ -f "$dir/template.tex" ]; then
        echo "Compiling: $dir (template.tex fallback — no preview-source.tex found)"
        (cd "$dir" && tectonic template.tex --outfmt pdf && mv template.pdf preview.pdf)
        echo "  -> preview.pdf generated from template.tex"
    fi
done
echo "Done. All template previews compiled."
