VizardCraft for macOS
==================

Install: drag VizardCraft.app onto the Applications folder.

First launch: this build is not notarized by Apple, so macOS may say it "cannot be opened
because the developer cannot be verified". Either

  * right-click (or Control-click) VizardCraft.app in Applications, choose Open, then Open again
    (needed once), or
  * run in Terminal:  xattr -dr com.apple.quarantine /Applications/VizardCraft.app

The VizardCraft Service (report portal) and command-line publisher are inside the app:

  /Applications/VizardCraft.app/Contents/MacOS/vizardcraft-server serve --port 8080 --token SECRET
  /Applications/VizardCraft.app/Contents/MacOS/vizardcraft-server export report.vizc --out site/

Documentation: docs/publishing.md in the source repository.
