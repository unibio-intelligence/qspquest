#include <R_ext/Rdynload.h>
extern void R_init_qspquest_extendr(DllInfo *dll);
void R_init_qspquest(DllInfo *dll) { R_init_qspquest_extendr(dll); }
