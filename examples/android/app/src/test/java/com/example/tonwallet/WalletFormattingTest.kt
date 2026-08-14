package com.example.tonwallet

import com.example.tonwallet.data.formatNanograms
import org.junit.Assert.assertEquals
import org.junit.Test

class WalletFormattingTest {
    @Test
    fun formatsExactNanogramAmounts() {
        assertEquals("0", formatNanograms("0"))
        assertEquals("1", formatNanograms("1000000000"))
        assertEquals("1.000000001", formatNanograms("1000000001"))
        assertEquals("0.01", formatNanograms("10000000"))
    }
}
