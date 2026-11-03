VizCraft for macOS
==================

Install: drag VizCraft.app onto the Applications folder.

First launch: this build is not notarized by Apple, so macOS may say it "cannot be opened
because the developer cannot be verified". Either

  * right-click (or Control-click) VizCraft.app in Applications, choose Open, then Open again
    (needed once), or
  * run in Terminal:  xattr -dr com.apple.quarantine /Applications/VizCraft.app

The VizCraft Service (report portal) and command-line publisher are inside the app:

  /Applications/VizCraft.app/Contents/MacOS/vizcraft-server serve --port 8080 --token SECRET
  /Applications/VizCraft.app/Contents/MacOS/vizcraft-server export report.vizc --out site/

Documentation: docs/publishing.md in the source repository.
