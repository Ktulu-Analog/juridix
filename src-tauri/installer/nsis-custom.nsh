; ── Messages personnalisés NSIS — Juridix ───────────────────────────────────

!define MUI_WELCOMEPAGE_TITLE "Bienvenue dans Juridix"
!define MUI_WELCOMEPAGE_TEXT "Cet assistant va installer Juridix sur votre ordinateur.$\r$\n$\r$\nJuridix est un assistant IA juridique, fonctionnant en local et communiquant avec les API Albert et Anthropic.$\r$\n$\r$\nCliquez sur Suivant pour continuer."

!define MUI_LICENSEPAGE_TEXT_TOP "Veuillez lire attentivement le contrat de licence avant d'installer Juridix."
!define MUI_LICENSEPAGE_TEXT_BOTTOM "Si vous acceptez les termes de ce contrat, cochez la case ci-dessous."
!define MUI_LICENSEPAGE_CHECKBOX
!define MUI_LICENSEPAGE_CHECKBOX_TEXT "J'accepte les termes du contrat de licence."

!define MUI_DIRECTORYPAGE_TEXT_TOP "Choisissez le dossier d'installation de Juridix."
!define MUI_DIRECTORYPAGE_TEXT_DESTINATION "Dossier de destination :"

!define MUI_INSTFILESPAGE_FINISHHEADER_TEXT "Installation terminée"
!define MUI_INSTFILESPAGE_FINISHHEADER_SUBTEXT "Juridix a été installé avec succès."
!define MUI_INSTFILESPAGE_ABORTHEADER_TEXT "Installation interrompue"
!define MUI_INSTFILESPAGE_ABORTHEADER_SUBTEXT "L'installation n'a pas abouti."

!define MUI_FINISHPAGE_TITLE "Juridix est prêt !"
!define MUI_FINISHPAGE_TEXT "L'installation de Juridix est terminée.$\r$\n$\r$\nCliquez sur Terminer pour fermer cet assistant."
!define MUI_FINISHPAGE_RUN "$INSTDIR\Juridix.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Lancer Juridix maintenant"
!define MUI_FINISHPAGE_LINK "github.com/Ktulu-Analog/juridix"
!define MUI_FINISHPAGE_LINK_LOCATION "https://github.com/Ktulu-Analog/juridix"

!define MUI_UNCONFIRMPAGE_TEXT_TOP "Juridix va être désinstallé de votre ordinateur."
!define MUI_UNCONFIRMPAGE_TEXT_LOCATION "Dossier à désinstaller :"
