package com.example.tonwallet

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.lifecycle.viewmodel.compose.viewModel
import com.example.tonwallet.ui.TonWalletApp
import com.example.tonwallet.ui.theme.TONWalletTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            TONWalletTheme {
                val walletViewModel: WalletViewModel = viewModel()
                TonWalletApp(walletViewModel)
            }
        }
    }
}
